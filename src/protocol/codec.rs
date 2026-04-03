use uuid::Uuid;

/// First byte of every encrypted binary WS frame identifies the payload type.
pub const FRAME_TYPE_DATA: u8 = 0x00;
pub const FRAME_TYPE_CONTROL: u8 = 0x01;

/// Data frame layout (after type byte):
///   [16 bytes: transfer UUID][8 bytes: offset BE][N bytes: chunk data]
const DATA_HEADER_SIZE: usize = 24; // UUID + offset

/// Encode a data chunk frame: [0x00][UUID][offset][data]
pub fn encode_data_frame(transfer_id: Uuid, offset: u64, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + DATA_HEADER_SIZE + data.len());
    frame.push(FRAME_TYPE_DATA);
    frame.extend_from_slice(transfer_id.as_bytes());
    frame.extend_from_slice(&offset.to_be_bytes());
    frame.extend_from_slice(data);
    frame
}

/// Encode a control message frame: [0x01][JSON bytes]
pub fn encode_control_frame(json: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + json.len());
    frame.push(FRAME_TYPE_CONTROL);
    frame.extend_from_slice(json);
    frame
}

/// Decode the type byte from a frame and return the payload.
pub fn decode_frame_type(frame: &[u8]) -> Result<(u8, &[u8]), CodecError> {
    if frame.is_empty() {
        return Err(CodecError::FrameTooShort);
    }
    Ok((frame[0], &frame[1..]))
}

/// Decode a data frame payload (after the type byte has been stripped).
pub fn decode_data_frame(payload: &[u8]) -> Result<(Uuid, u64, &[u8]), CodecError> {
    if payload.len() < DATA_HEADER_SIZE {
        return Err(CodecError::FrameTooShort);
    }

    let uuid = Uuid::from_slice(&payload[..16]).map_err(|_| CodecError::InvalidUuid)?;
    let offset = u64::from_be_bytes(payload[16..24].try_into().unwrap());
    let data = &payload[24..];

    Ok((uuid, offset, data))
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame too short")]
    FrameTooShort,
    #[error("invalid UUID")]
    InvalidUuid,
    #[allow(dead_code)]
    #[error("unknown frame type: {0:#x}")]
    UnknownType(u8),
}
