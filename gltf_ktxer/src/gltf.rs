use std::{collections::HashMap, hash::Hash, marker::PhantomData, ops::Index, slice::SliceIndex, u64};

use crate::{Error, Result};

use base64::prelude::*;
use serde_derive::{Deserialize, Serialize};

pub type GltfDoc = serde_json::Map<String, serde_json::Value>;

/// A wrapper for u64 that uses the maximum value as a sentinel for not-set.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct GltfIndex<T>(usize, PhantomData<T>);
impl<T> GltfIndex<T> {
    pub fn of(x: usize) -> Self {
        Self(x, PhantomData::default())
    }
    pub fn exists(&self) -> bool {
        self.0 != usize::MAX
    }
}
impl<T> From<usize> for GltfIndex<T> {
    fn from(value: usize) -> Self {
        GltfIndex::of(value)
    }
}
impl<T> Default for GltfIndex<T> {
    fn default() -> Self {
        GltfIndex::of(usize::MAX)
    }
}
impl<T> PartialEq for GltfIndex<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T> Eq for GltfIndex<T> {}
impl<T> Hash for GltfIndex<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}


pub trait GltfList<T> : std::ops::Index<usize, Output = T> + Sized {
    fn gltf_index(&self, idx: GltfIndex<T>, list_name: &'static str) -> Result<Option<&T>>;
    fn gltf_index_required(&self, idx: GltfIndex<T>, list_name: &'static str) -> Result<&T> {
        match self.gltf_index(idx, list_name)? {
            None => Err(Error::IdxNotSet { list_name }),
            Some(data) => Ok(data),
        }
    }
}
impl<T> GltfList<T> for Vec<T> {
    fn gltf_index(&self, idx: GltfIndex<T>, list_name: &'static str) -> Result<Option<&T>> {
        if idx.0 == usize::MAX {
            Ok(None)
        } else if idx.0 > self.len() {
            Err(Error::IdxOOB { list_name, idx: idx.0, num: self.len() })
        } else {
            Ok(Some(&self[idx.0]))
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GltfUri(String);

/// A buffer points to binary geometry, animation, or skins.
#[derive(Serialize, Deserialize, Debug)]
pub struct GltfBuffer {
    /// The URI (or IRI) of the buffer.
    /// Relative paths are relative to the current glTF asset.
    /// Instead of referencing an external file, this field **MAY** contain a `data:`-URI.
    /// It may also be None if referencing a KTX
    pub uri: Option<GltfUri>,
    /// The length of the buffer in bytes.
    #[serde(rename = "byteLength")]
    pub byte_length: usize,
    pub name: serde_json::Value,
    pub extensions: serde_json::Value,
    pub extras: serde_json::Value,
}

/// A view into a buffer generally representing a subset of the buffer.
#[derive(Serialize, Deserialize, Debug)]
pub struct GltfBufferView {
    /// The index of the buffer.
    pub buffer: GltfIndex<GltfBuffer>,
    /// The offset into the buffer in bytes.
    #[serde(rename = "byteOffset", default)]
    pub byte_offset: usize,
    /// The length of the bufferView in bytes.
    #[serde(rename = "byteLength")]
    pub byte_length: usize,
    /// The stride, in bytes, between vertex attributes.
    /// When this is not defined, data is tightly packed.
    /// When two or more accessors use the same buffer view, this field **MUST** be defined.
    #[serde(rename = "byteStride")]
    pub byte_stride: Option<usize>,
    /// The hint representing the intended GPU buffer type to use with this buffer view.
    pub target: Option<u64>,
    pub name: serde_json::Value,
    pub extensions: serde_json::Value,
    pub extras: serde_json::Value,
}

#[derive(Debug)]
struct GltfSampler();

/// A texture and its sampler.
#[derive(Serialize, Deserialize, Debug)]
pub struct GltfTexture {
    /// The index of the sampler used by this texture.
    /// When undefined, a sampler with repeat wrapping and auto filtering **SHOULD** be used.
    pub sampler: Option<GltfIndex<GltfSampler>>,
    /// The index of the image used by this texture.
    /// When undefined, an extension or other mechanism **SHOULD** supply an alternate texture source, otherwise behavior is undefined.
    pub source: Option<GltfIndex<GltfImage>>,
    pub name: serde_json::Value,
    pub extensions: serde_json::Value,
    pub extras: serde_json::Value,
}

/// Image data used to create a texture. Image **MAY** be referenced by an URI (or IRI) or a buffer view index.
#[derive(Serialize, Deserialize, Debug)]
pub struct GltfImage {
    /// The URI (or IRI) of the image.
    /// Relative paths are relative to the current glTF asset.
    /// Instead of referencing an external file, this field **MAY** contain a `data:`-URI.
    /// This field **MUST NOT** be defined when `bufferView` is defined.
    pub uri: Option<GltfUri>,
    /// The image's media type.
    /// This field **MUST** be defined when `bufferView` is defined.
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    /// The index of the bufferView that contains the image.
    /// This field **MUST NOT** be defined when `uri` is defined.
    #[serde(rename = "bufferView", default)]
    pub buffer_view: GltfIndex<GltfBufferView>,
    pub name: serde_json::Value,
    pub extensions: serde_json::Value,
    pub extras: serde_json::Value,
}

pub enum U8VecOrSlice<'a> {
    V(Vec<u8>),
    S(&'a [u8]),
}
impl<'a> U8VecOrSlice<'a> {
    fn of_sliced_vec(v: &'a Vec<u8>, len: usize) -> Result<U8VecOrSlice<'a>> {
        if len > v.len() {
            Err(Error::BufferNotLongEnough { expected_bytes: len, got_bytes: v.len() })
        } else {
            Ok(U8VecOrSlice::S(&v[0..len]))
        }
    }
    fn of_owned_vec(mut v: Vec<u8>, len: usize) -> Result<U8VecOrSlice<'a>> {
        if len > v.len() {
            Err(Error::BufferNotLongEnough { expected_bytes: len, got_bytes: v.len() })
        } else {
            for _ in 0..(len-v.len()) {
                v.pop();
            }
            Ok(U8VecOrSlice::V(v))
        }
    }
    pub fn len(&self) -> usize {
        match self {
            U8VecOrSlice::V(items) => items.len(),
            U8VecOrSlice::S(items) => items.len(),
        }
    }
}
impl<'a, I: SliceIndex<[u8]>> Index<I> for U8VecOrSlice<'a> {
    type Output = I::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        match self {
            U8VecOrSlice::V(items) => &items[index],
            U8VecOrSlice::S(items) => &items[index],
        }
    }
}

/// Extract the base64-encoded part of a value glTF2.0 buffer data URI, returning None if the URI is not a valid base64 data URI.
/// 
/// 1. glTF2.0 section 2.8: 
/// "Data URIs that embed binary resources in the glTF JSON as defined by the RFC 2397. The Data URI’s mediatype field MUST match the encoded content."
/// 
/// 2. glTF2.0 section 3.6.1.1:
/// "Buffer data MAY alternatively be embedded in the glTF file via data: URI with base64 encoding.
/// When data: URI is used for buffer storage, its mediatype field MUST be set to application/octet-stream or application/gltf-buffer."
/// 
/// 3. RFC 2397:
/// ```notest
/// 3. Syntax
/// 
/// dataurl    := "data:" [ mediatype ] [ ";base64" ] "," data
/// mediatype  := [ type "/" subtype ] *( ";" parameter )
/// data       := *urlchar
/// parameter  := attribute "=" value
/// 
/// where "urlchar" is imported from [RFC2396], and "type", "subtype",
/// "attribute" and "value" are the corresponding tokens from [RFC2045],
/// represented using URL escaped encoding of [RFC2396] as necessary.
/// 
/// Attribute values in [RFC2045] are allowed to be either represented as
/// tokens or as quoted strings. However, within a "data" URL, the
/// "quoted-string" representation would be awkward, since the quote mark
/// is itself not a valid urlchar. For this reason, parameter values
/// should use the URL Escaped encoding instead of quoted string if the
/// parameter values contain any "tspecial".
/// 
/// The ";base64" extension is distinguishable from a content-type
/// parameter by the fact that it doesn't have a following "=" sign.
/// ```
fn base64str_from_data_uri(uri: &str) -> Option<&str> {
    // data always at the start[3]
    let uri = uri.strip_prefix("data:")?;
    // mediatype always has to be defined as one of exactly two choices, without key-value parameters[2]
    let uri = {
        // try octet-stream
        if let Some(uri) = uri.strip_prefix("application/octet-stream") {
            uri
        } else {
            // try gltf-buffer, return None if neither
            uri.strip_prefix("application/gltf-buffer")?
        }
    };
    // optionally has ";base64", always has comma
    uri.strip_prefix(";base64,").or_else(|| uri.strip_prefix(","))
}

pub fn dump_data<'a>(uri: &Option<GltfUri>, idx: usize, map: &'a HashMap<Option<String>, Vec<u8>>, byte_length: usize) -> Result<U8VecOrSlice<'a>> {
    match uri {
        None if idx == 0 => match map.get(&None) {
            Some(data) => U8VecOrSlice::of_sliced_vec(data, byte_length),
            None => Err(Error::BufferUriMissingData(None))
        }
        None => Err(Error::BufferHadNoUri(idx)),
        Some(uri) => {
            if let Some(data) = base64str_from_data_uri(uri.0.as_str()) {
                // RFC 2397 for data URIs contains an example in section 4
                // which uses the '/' character. While the base64 crate does have a URL-safe alphabet which avoids + and /, 
                // we can assume we don't need to use it.
                U8VecOrSlice::of_owned_vec(BASE64_STANDARD.decode(data)?, byte_length)
            } else {
                match map.get(&Some(uri.0.clone())) {
                    Some(data) => U8VecOrSlice::of_sliced_vec(data, byte_length),
                    None => Err(Error::BufferUriMissingData(Some(uri.0.clone())))
                }
            }
        }
    }
}
