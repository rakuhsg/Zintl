//! Canonical `ZJE1` engine-event decoder.

use crate::{
    EngineError, EngineEvent, EvaluationId, EvaluationOutcome, FilesystemRequest, HostRequest,
    HostRequestId, JavaScriptException,
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
        10 => HostRequest::Filesystem(FilesystemRequest::ReadFile {
            maximum_bytes: usize::try_from(cursor.u64()?)
                .map_err(|_| EngineError::QuotaExceeded)?,
            url: utf8_remaining(&mut cursor)?,
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
    use crate::{EngineEvent, EvaluationId, EvaluationOutcome};

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
}
