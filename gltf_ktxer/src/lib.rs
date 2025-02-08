use std::{collections::{HashMap, HashSet}, num::NonZeroU8};

use gltf::{dump_data, GltfBuffer, GltfBufferView, GltfDoc, GltfImage, GltfIndex, GltfList, GltfTexture, U8VecOrSlice};
// use libktx_rs::{sources::{CommonCreateInfo, Ktx2CreateInfo}, sys::ktxStream, TextureSource};

mod gltf;
mod error;
use error::{Error, Result};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

struct Input<'a> {
    gltf_json: &'a mut GltfDoc,
    binaries: &'a HashMap<Option<String>, Vec<u8>>,
}
impl<'a> Input<'a> {
    fn get_list<T: DeserializeOwned>(&self, name: &str) -> Result<Vec<T>> {
        match self.gltf_json.get(name) {
            None => Ok(vec![]),
            Some(value) => Ok(serde_json::from_value(value.clone())?)
        }
    }
    fn set_list<T: Serialize>(&mut self, name: &str, data: Vec<T>) -> Result<()> {
        self.gltf_json.insert(name.to_string(), serde_json::to_value(data)?);
        Ok(())
    }
    fn consume_doc(mut self) -> GltfDoc {
        std::mem::take(&mut self.gltf_json)
    }
}

struct Output {
    gltf_json: GltfDoc,
    binary: Vec<u8>,
}



fn pack_buffers_together(mut input: Input<'_>) -> Result<Output> {
    let buffers: Vec<GltfBuffer> = input.get_list("buffers")?;
    let buffer_views: Vec<GltfBufferView> = input.get_list("bufferViews")?;

    let buffer_datas: Vec<U8VecOrSlice<'_>> = buffers
        .into_iter()
        .enumerate()
        .map(|(idx, b)| dump_data(&b.uri, idx, input.binaries, b.byte_length))
        .collect::<Result<_>>()?;
    let (new_buffer_views, new_buffer) = pack_buffer_views(
        buffer_views.into_iter().map(|v| {
            let buffer = buffer_datas.gltf_index_required(v.buffer, "buffers")?;
            if v.byte_offset + v.byte_length > buffer.len() {
                Err(Error::BufferViewSizeOOB { buffer_len: buffer.len(), buffer_view_off: v.byte_offset, buffer_view_len: v.byte_length })
            } else {
                let data = &buffer[v.byte_offset..(v.byte_offset+v.byte_length)];
                Ok((v, data))
            }
        })
    )?;

    input.set_list("buffers", vec![
        GltfBuffer {
            uri: None,
            byte_length: new_buffer.len(),
            name: serde_json::Value::Null,
            extensions: serde_json::Value::Null,
            extras: serde_json::Value::Null,
        }
    ])?;
    input.set_list("bufferViews", new_buffer_views)?;
    Ok(Output { gltf_json: input.consume_doc(), binary: new_buffer, })
}

fn pack_buffer_views<'a, I>(iter: I) -> Result<(Vec<GltfBufferView>, Vec<u8>)>
    where I: IntoIterator<Item = Result<(GltfBufferView, &'a [u8])>>
{
    let mut new_buffer_views = vec![];
    let mut new_buffer = vec![];

    for item in iter {
        match item {
            Ok((buffer_view, data)) => {
                new_buffer_views.push(
                    GltfBufferView {
                        buffer: 0.into(),
                        byte_offset: data.len(),
                        ..buffer_view
                    }
                );
                new_buffer.extend_from_slice(data);
                // Pad out the new_buffer to be 4-byte aligned.
                // Section 3.6.2.4 https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#data-alignment
                // requires accessor.byteOffset and (accessor.byteOffset + bufferView.byteOffset) to 
                // always be a multiple of the size of the accessor's component type.
                // the maximum component type size is 4 (32 bits, as seen in 3.6.2.2 Accessor Data Types).
                // therefore always pad out to 4-bytes to be sure we're always aligned.
                new_buffer.extend_from_slice(&[0xFF; 4 - data.len() % 4]);
                assert!(data.len() % 4 == 0);
            }
            Err(e) => return Err(e)
        } 
        
    }

    Ok((new_buffer_views, new_buffer))
}

// enum BasicImageFormat {
//     RGB,
//     RGBA,
// }
// struct BasicImageData<'a> {
//     fmt: BasicImageFormat,
//     bytes: &'a [u8],
//     export_as_srgb: bool,
// }

fn texture_ktx_source(image: &GltfTexture) -> Option<GltfIndex<GltfImage>> {
    image
        .extensions
        .as_object()?
        .get("KHR_texture_basisu")?
        .as_object()?
        .get("source")?
        .as_u64()
        .map(|idx| GltfIndex::of(idx as usize))
}

fn material_diffuse_tex(mat: &serde_json::Value) -> Option<GltfIndex<GltfTexture>> {
    mat
        .as_object()?
        .get("pbrMetallicRoughness")?
        .as_object()?
        .get("baseColorTexture")?
        .as_object()?
        .get("index")?
        .as_u64()
        .map(|idx| GltfIndex::of(idx as usize))
}
fn material_emissive_tex(mat: &serde_json::Value) -> Option<GltfIndex<GltfTexture>> {
    mat
        .as_object()?
        .get("emissiveTexture")?
        .as_object()?
        .get("index")?
        .as_u64()
        .map(|idx| GltfIndex::of(idx as usize))
}

fn get_srgb_texture_indices(input: &Input) -> HashSet<GltfIndex<GltfTexture>> {
    let mut set = HashSet::new();
    if let Some(materials) = input.gltf_json.get("materials").and_then(|val| val.as_array()) {
        for mat in materials {
            if let Some(diffuse) = material_diffuse_tex(mat) {
                set.insert(diffuse);
            }
            if let Some(emissive) = material_emissive_tex(mat) {
                set.insert(emissive);
            }
        }
    }
    set
}

struct ReencodeJobs {
    new_textures: Vec<GltfTexture>,
    new_images: Vec<ImageReencodeJob>,
}

enum ImageReencodeFormat {
    Basic(image::ImageFormat),
    // a KTX2 texture using basis compression
    Ktx {
        basis_compression_quality: Option<NonZeroU8>,
        transcoded_to_bc1_or_bc3: bool,
    }
}

struct Params {
    uncompressed_format: Option<image::ImageFormat>,
    ktx_basis_compression_quality: Option<NonZeroU8>,
    ktx_transcode_to_bc1_or_bc3: bool,
}
impl Default for Params {
    fn default() -> Self {
        Self {
            uncompressed_format: Some(image::ImageFormat::Jpeg),
            ktx_basis_compression_quality: None,
            ktx_transcode_to_bc1_or_bc3: true,
        }
    }
}

struct ImageReencodeJob {
    data: Vec<u8>,
    data_mime_type: Vec<u8>,
    data_used_as_srgb: bool,
    reencode_as: ImageReencodeFormat,
    preexisting_buffer_view_idx: GltfIndex<GltfBufferView>,
}

fn get_reencode_jobs(input: Input, params: Params) -> Result<ReencodeJobs> {
    let mut textures: Vec<GltfTexture> = input.get_list("textures")?;
    let images: Vec<GltfImage> = input.get_list("images")?;
    let srgb_texture_indices = get_srgb_texture_indices(&input);
    
    let mut new_images = vec![];
    let mut old_image_idx_to_new_image_idx = HashMap::new();
    let lookup_old_img = |old_img_idx: GltfIndex<GltfImage>, srgb: bool, reencode_as: ImageReencodeFormat| -> Result<GltfIndex<GltfImage>> {
        if let Some(new_img_idx) = old_image_idx_to_new_image_idx.get(&old_img_idx) {
            Ok(*new_img_idx)
        } else {
            let new_img_idx = GltfIndex::of(new_images.len());
            new_images.push(ImageReencodeJob {
                data: todo!(),
                data_mime_type: todo!(),
                data_used_as_srgb: srgb,
                reencode_as,
                preexisting_buffer_view_idx: images.gltf_index_required(old_img_idx, "images")?.buffer_view,
            });
            old_image_idx_to_new_image_idx.insert(old_img_idx, new_img_idx);
            Ok(new_img_idx)
        }
    };

    for (tex_idx, tex) in textures.iter().enumerate() {
        let data_used_as_srgb = srgb_texture_indices.contains(GltfIndex::of(tex_idx));
        let unoptimized_img = tex.source;
        let optimized_img = texture_ktx_source(tex);

        let mut new_texture = tex.clone();
        // TODO extract source data from unoptimized_img or optimized_img
        // TODO make new images for each of (unoptimized) and (optimized) new images, potentially reusing the buffer views from the old ones
    }
}

/*
fn parse_and_reencode(input: Input) -> Result<Output> {
    // let glb = gltf::Gltf::from_slice(input)?;
    // let (gltf, binary) = match glb.blob {
    //     Some(blob) => (&mut glb.document, &mut blob),
    //     None => return Error::Gltf(gltf::Error::MissingBlob),
    // };

    // let mut image_views = vec![];
    // let mut unembedded_image_views = vec![];
    // for image in gltf.images() {
    //     let ktx_source_idx = ktx_source(&image);
    //     match image.source() {
    //         gltf::image::Source::View { view, mime_type } => {
    //             image_views.push((image.index(), view, mime_type, ktx_source_idx))
    //         }
    //         gltf::image::Source::Uri { .. } => unembedded_image_views.push((image.index(), image.name())),
    //     }
    // }
    let srgb_image_idxs = get_srgb_images(&input);
    let mut images: Vec<GltfImage> = input.get_list("images")?;
    for image in images {

    }
    

    for (image_idx, uncompressed_view, uncompressed_type, ktx_source_idx) in image_views {
        let buf = view_data(binary, &uncompressed_view);
        let image_data = match ImageFormat::from_mime_type(uncompressed_type) {
            Some(format) => image::load_from_memory_with_format(buf, format)?,
            None => image::load_from_memory(buf)?
        };
        let (vk_format, rgb_bytes) = if image_data.color().has_alpha() {
            (
                43, // VK_FORMAT_R8G8B8A8_SRGB 
                image_data.into_rgba8(),
            )
        } else {
            (
                29, // VK_FORMAT_R8G8B8_SRGB 
                image_data.into_rgb8(),
            )
        };

        let info = Ktx2CreateInfo {
            vk_format,
            dfd: None,
            common: CommonCreateInfo {
                create_storage: libktx_rs::CreateStorage::AllocStorage,
                base_width: image_data.width(),
                base_height: image_data.height(),
                base_depth: 1,
                num_dimensions: 2,
                num_levels: 1,
                num_faces: 1,
                num_layers: 1,
                is_array: false,
                generate_mipmaps: false, // TODO make param
            }
        };

        let mut ktx = info.create_texture()?;
        unsafe {
            (*(*ktx.handle()).vtbl).SetImageFromMemory(ktx.handle(), 0, 0, 0, rgb_bytes.as_ptr(), rgb_bytes.len());
        }
        ktx.ktx2().unwrap().compress_basis(params.ktx_basis_compression_quality.into()); // TODO make param
        match params.ktx_transcode_to {
            Some(format) => {
                // TODO check if this is smaller or larger
                ktx.ktx2().unwrap().transcode_basis(format, libktx_rs::TranscodeFlags::HIGH_QUALITY);
            }
            None => {}
        }
        let ktx_bytes = ktx.write_to(sink)?;

        let ktx_source_idx = match ktx_source_idx {
            Some(ktx_source_idx) => ktx_source_idx,
            None => {
                // TDOO add new gltf source
                // TODO point image at new source
                // KHR_texture_basisu
                gltf.views()
            }
        };
        // TODO add data to binary blob
    }
}
    */