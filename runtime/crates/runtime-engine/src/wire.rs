//! Canonical `ZJE1` engine-event decoder.

use crate::{
    EngineError, EngineEvent, EvaluationId, EvaluationOutcome, HostRequest, HostRequestId,
    JavaScriptException, MountRequest,
};

/// Decodes one complete canonical engine event.
///
/// # Errors
///
/// Rejects unknown versions/kinds, truncation, invalid UTF-8 and trailing bytes.
pub fn decode_event(bytes: &[u8]) -> Result<EngineEvent, EngineError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(4)? != b"ZJE1" || cursor.u16()? != 1 {
        return Err(EngineError::Backend);
    }
    let kind = cursor.u16()?;
    let id = cursor.u64()?;
    let event = match kind {
        1 => EngineEvent::EvaluationSettled {
            id: EvaluationId(id),
            outcome: EvaluationOutcome::Value(cursor.length_prefixed()?.to_vec()),
        },
        2 => {
            let value =
                std::str::from_utf8(cursor.length_prefixed()?).map_err(|_| EngineError::Backend)?;
            let (name, message) = value.split_once(':').unwrap_or(("Error", value));
            EngineEvent::EvaluationSettled {
                id: EvaluationId(id),
                outcome: EvaluationOutcome::Exception(JavaScriptException {
                    name: name.trim().to_owned(),
                    message: message.trim().to_owned(),
                }),
            }
        }
        3 => {
            let _ = cursor.length_prefixed()?;
            EngineEvent::EvaluationSettled {
                id: EvaluationId(id),
                outcome: EvaluationOutcome::Cancelled,
            }
        }
        4 => {
            let operation = cursor.u16()?;
            let payload = cursor.length_prefixed()?;
            EngineEvent::HostRequest {
                id: HostRequestId(id),
                request: decode_host_request(operation, payload)?,
            }
        }
        5 if id == 0 => EngineEvent::ConsoleOutput(cursor.length_prefixed()?.to_vec()),
        _ => return Err(EngineError::Backend),
    };
    if cursor.is_empty() {
        Ok(event)
    } else {
        Err(EngineError::Backend)
    }
}

fn decode_host_request(kind: u16, payload: &[u8]) -> Result<HostRequest, EngineError> {
    let mut cursor = Cursor::new(payload);
    let request = match kind {
        1 => {
            let version = cursor.u32()?;
            let name_length = usize::from(cursor.u16()?);
            let name = std::str::from_utf8(cursor.take(name_length)?)
                .map_err(|_| EngineError::Backend)?
                .to_owned();
            HostRequest::Invoke {
                name,
                version,
                input: cursor.take_remaining().to_vec(),
            }
        }
        2 => HostRequest::Sleep {
            nanoseconds: cursor.u64()?,
        },
        10 => HostRequest::Mount(MountRequest::ReadFile {
            maximum_bytes: usize::try_from(cursor.u64()?)
                .map_err(|_| EngineError::QuotaExceeded)?,
            url: utf8_remaining(&mut cursor)?,
        }),
        11 => {
            let url = cursor.length_prefixed_utf8_u16()?;
            HostRequest::Mount(MountRequest::WriteFile {
                url,
                bytes: cursor.take_remaining().to_vec(),
            })
        }
        12 => HostRequest::Mount(MountRequest::CreateDirectory {
            url: utf8_remaining(&mut cursor)?,
        }),
        13 => HostRequest::Mount(MountRequest::RemoveFile {
            url: utf8_remaining(&mut cursor)?,
        }),
        14 => HostRequest::Mount(MountRequest::RemoveDirectory {
            url: utf8_remaining(&mut cursor)?,
        }),
        15 => HostRequest::Mount(MountRequest::Rename {
            from: cursor.length_prefixed_utf8_u16()?,
            to: utf8_remaining(&mut cursor)?,
        }),
        _ => return Err(EngineError::Backend),
    };
    if cursor.is_empty() {
        Ok(request)
    } else {
        Err(EngineError::Backend)
    }
}

fn utf8_remaining(cursor: &mut Cursor<'_>) -> Result<String, EngineError> {
    std::str::from_utf8(cursor.take_remaining())
        .map(str::to_owned)
        .map_err(|_| EngineError::Backend)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], EngineError> {
        let end = self.offset.checked_add(count).ok_or(EngineError::Backend)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(EngineError::Backend)?;
        self.offset = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, EngineError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().map_err(|_| EngineError::Backend)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, EngineError> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().map_err(|_| EngineError::Backend)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, EngineError> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| EngineError::Backend)?,
        ))
    }
    fn length_prefixed_utf8_u16(&mut self) -> Result<String, EngineError> {
        let length = usize::from(self.u16()?);
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| EngineError::Backend)
    }
    fn length_prefixed(&mut self) -> Result<&'a [u8], EngineError> {
        let length = usize::try_from(self.u32()?).map_err(|_| EngineError::Backend)?;
        self.take(length)
    }
    fn take_remaining(&mut self) -> &'a [u8] {
        let value = &self.bytes[self.offset..];
        self.offset = self.bytes.len();
        value
    }
    const fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::decode_event;
    use crate::{
        EngineEvent, EvaluationId, EvaluationOutcome, HostRequest, HostRequestId, MountRequest,
    };

    #[test]
    // Verifies the canonical value vector decodes without accepting trailing bytes.
    fn canonical_evaluation_vector_decodes() {
        let mut bytes = b"ZJE1\0\x01\0\x01\0\0\0\0\0\0\0\x01\0\0\0\x02{}".to_vec();
        assert_eq!(
            decode_event(&bytes),
            Ok(EngineEvent::EvaluationSettled {
                id: EvaluationId(1),
                outcome: EvaluationOutcome::Value(b"{}".to_vec()),
            })
        );
        bytes.push(0);
        assert!(decode_event(&bytes).is_err());
    }

    #[test]
    // Verifies console events require the reserved zero identity and canonical framing.
    fn canonical_console_vector_decodes() {
        let bytes = b"ZJE1\0\x01\0\x05\0\0\0\0\0\0\0\0\0\0\0\x05hello";
        assert_eq!(
            decode_event(bytes),
            Ok(EngineEvent::ConsoleOutput(b"hello".to_vec()))
        );
        let mut invalid = bytes.to_vec();
        invalid[15] = 1;
        assert!(decode_event(&invalid).is_err());
    }

    #[test]
    // Verifies mount mutation requests decode into typed data without closures or OS handles.
    fn canonical_mount_write_vector_decodes() {
        let url = b"mount://project/demo.txt";
        let mut payload = Vec::new();
        payload.extend_from_slice(&u16::try_from(url.len()).unwrap().to_be_bytes());
        payload.extend_from_slice(url);
        payload.extend_from_slice(b"hello");
        let mut bytes = b"ZJE1\0\x01\0\x04\0\0\0\0\0\0\0\x09\0\x0b".to_vec();
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&payload);
        assert_eq!(
            decode_event(&bytes),
            Ok(EngineEvent::HostRequest {
                id: HostRequestId(9),
                request: HostRequest::Mount(MountRequest::WriteFile {
                    url: "mount://project/demo.txt".into(),
                    bytes: b"hello".to_vec(),
                }),
            })
        );
    }
}
