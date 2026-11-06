use crate::LoadError;

pub(crate) fn write_name(bytes: &mut Vec<u8>, name: &str) {
    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(name.as_bytes());
}

pub(crate) fn take<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = cursor.checked_add(len)?;
    let value = bytes.get(*cursor..end)?;
    *cursor = end;
    Some(value)
}

pub(crate) fn read_u16(bytes: &[u8], cursor: &mut usize) -> Option<u16> {
    Some(u16::from_le_bytes(take(bytes, cursor, 2)?.try_into().ok()?))
}

pub(crate) fn read_u32(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    Some(u32::from_le_bytes(take(bytes, cursor, 4)?.try_into().ok()?))
}

pub(crate) fn read_u64(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    Some(u64::from_le_bytes(take(bytes, cursor, 8)?.try_into().ok()?))
}

pub(crate) fn read_name(bytes: &[u8], cursor: &mut usize) -> Result<String, LoadError> {
    let len = read_u32(bytes, cursor)
        .ok_or_else(|| LoadError::InvalidManifest("truncated name length".into()))?;
    let raw = take(bytes, cursor, len as usize)
        .ok_or_else(|| LoadError::InvalidManifest("truncated name".into()))?;
    String::from_utf8(raw.to_vec())
        .map_err(|_| LoadError::InvalidManifest("manifest name is not UTF-8".into()))
}
