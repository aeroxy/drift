use uuid::Uuid;

#[allow(dead_code)]
const HEADER_SIZE: usize = 24; // 16 bytes UUID + 8 bytes offset

#[allow(dead_code)]
pub fn encode_data_frame(transfer_id: Uuid, offset: u64, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(HEADER_SIZE + data.len());
    frame.extend_from_slice(transfer_id.as_bytes());
    frame.extend_from_slice(&offset.to_be_bytes());
    frame.extend_from_slice(data);
    frame
}

#[allow(dead_code)]
pub fn decode_data_frame(frame: &[u8]) -> Result<(Uuid, u64, &[u8]), CodecError> {
    if frame.len() < HEADER_SIZE {
        return Err(CodecError::FrameTooShort);
    }

    let uuid = Uuid::from_slice(&frame[..16]).map_err(|_| CodecError::InvalidUuid)?;
    let offset = u64::from_be_bytes(frame[16..24].try_into().unwrap());
    let data = &frame[24..];

    Ok((uuid, offset, data))
}

#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame too short")]
    FrameTooShort,
    #[error("invalid UUID")]
    InvalidUuid,
}
