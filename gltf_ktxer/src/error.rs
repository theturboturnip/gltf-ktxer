use thiserror::Error;

#[derive(Error)]
pub enum Error {
    // Gltf(#[from] gltf::Error),
    // Ktx(#[from] KtxError),
    Image(#[from] image::Error),
    Serde(#[from] serde_json::Error),
    BufferHadNoUri(usize),
    BufferUriMissingData(Option<String>),
    BufferUriBadBase64(#[from] base64::DecodeError),
    BufferNotLongEnough {
        expected_bytes: usize,
        got_bytes: usize,
    },
    BufferViewSizeOOB {
        buffer_len: usize,
        buffer_view_off: usize,
        buffer_view_len: usize,
    },
    IdxNotSet {
        list_name: &'static str,
    },
    IdxOOB {
        list_name: &'static str,
        idx: usize,
        num: usize,
    },
    ExpectedList {
        key: &'static str,
    }
}

pub type Result<T> = std::result::Result<T, Error>;