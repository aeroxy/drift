use uuid::Uuid;

/// First byte of every encrypted binary WS frame identifies the payload type.
pub const FRAME_TYPE_DATA: u8 = 0x00; // V1: [UUID][offset] (no file_index)
pub const FRAME_TYPE_DATA_V2: u8 = 0x02; // V2: [UUID][file_index][offset]
pub const FRAME_TYPE_CONTROL: u8 = 0x01;

/// Data frame layout (after type byte):
///   V1: [16 bytes: transfer UUID][8 bytes: offset BE][N bytes: chunk data]
///   V2: [16 bytes: transfer UUID][4 bytes: file_index BE][8 bytes: offset BE][N bytes: chunk data]
const DATA_HEADER_SIZE_V1: usize = 24; // UUID + offset
const DATA_HEADER_SIZE_V2: usize = 28; // UUID + file_index + offset

/// Encode a V1 data chunk frame: [0x00][UUID][offset][data]
pub fn encode_data_frame_v1(transfer_id: Uuid, offset: u64, data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + DATA_HEADER_SIZE_V1 + data.len());
    frame.push(FRAME_TYPE_DATA);
    frame.extend_from_slice(transfer_id.as_bytes());
    frame.extend_from_slice(&offset.to_be_bytes());
    frame.extend_from_slice(data);
    frame
}

/// Encode a V2 data chunk frame: [0x02][UUID][file_index][offset][data]
pub fn encode_data_frame_v2(
    transfer_id: Uuid,
    file_index: u32,
    offset: u64,
    data: &[u8],
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + DATA_HEADER_SIZE_V2 + data.len());
    frame.push(FRAME_TYPE_DATA_V2);
    frame.extend_from_slice(transfer_id.as_bytes());
    frame.extend_from_slice(&file_index.to_be_bytes());
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
/// For V1 frames (FRAME_TYPE_DATA), file_index is always 0.
/// For V2 frames (FRAME_TYPE_DATA_V2), file_index is extracted from the header.
pub fn decode_data_frame(
    payload: &[u8],
    frame_type: u8,
) -> Result<(Uuid, u32, u64, &[u8]), CodecError> {
    match frame_type {
        FRAME_TYPE_DATA => {
            // V1 format: [UUID 16B][offset 8B][data]
            if payload.len() < DATA_HEADER_SIZE_V1 {
                return Err(CodecError::FrameTooShort);
            }
            let uuid = Uuid::from_slice(&payload[..16]).map_err(|_| CodecError::InvalidUuid)?;
            let offset = u64::from_be_bytes(payload[16..24].try_into().unwrap());
            let data = &payload[24..];
            Ok((uuid, 0, offset, data))
        }
        FRAME_TYPE_DATA_V2 => {
            // V2 format: [UUID 16B][file_index 4B][offset 8B][data]
            if payload.len() < DATA_HEADER_SIZE_V2 {
                return Err(CodecError::FrameTooShort);
            }
            let uuid = Uuid::from_slice(&payload[..16]).map_err(|_| CodecError::InvalidUuid)?;
            let file_index = u32::from_be_bytes(payload[16..20].try_into().unwrap());
            let offset = u64::from_be_bytes(payload[20..28].try_into().unwrap());
            let data = &payload[28..];
            Ok((uuid, file_index, offset, data))
        }
        _ => Err(CodecError::UnknownType(frame_type)),
    }
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
