//! Bounded versioned wire envelopes. Payload schemas are interpreted by ops.

const MAGIC: [u8; 4] = *b"ZRT1";
pub const PROTOCOL_VERSION: u16 = 1;
const REQUEST_KIND: u16 = 1;
const COMPLETION_KIND: u16 = 2;
const REQUEST_HEADER: usize = 24;
const COMPLETION_HEADER: usize = 28;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestEnvelope {
    pub request_id: u64,
    pub op_id: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionEnvelope {
    pub request_id: u64,
    pub status: u32,
    pub payload: Vec<u8>,
}

/// Encodes a request into the stable network-byte-order envelope.
///
/// # Errors
///
/// Rejects payloads that cannot be represented by the protocol length field.
pub fn encode_request(request: &RequestEnvelope) -> Result<Vec<u8>, CodecError> {
    let payload_len =
        u32::try_from(request.payload.len()).map_err(|_| CodecError::LengthOverflow)?;
    let mut output = Vec::with_capacity(REQUEST_HEADER + request.payload.len());
    output.extend_from_slice(&MAGIC);
    output.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    output.extend_from_slice(&REQUEST_KIND.to_be_bytes());
    output.extend_from_slice(&request.request_id.to_be_bytes());
    output.extend_from_slice(&request.op_id.to_be_bytes());
    output.extend_from_slice(&payload_len.to_be_bytes());
    output.extend_from_slice(&request.payload);
    Ok(output)
}

/// Decodes a request after checking version, kind, exact length, IDs, and limit.
///
/// # Errors
///
/// Rejects truncated, trailing, oversized, unknown-version, wrong-kind, or
/// zero-identity input without allocation proportional to an untrusted length.
pub fn decode_request(input: &[u8], max_payload: usize) -> Result<RequestEnvelope, CodecError> {
    validate_prefix(input, REQUEST_HEADER, REQUEST_KIND)?;
    let request_id = read_u64(input, 8)?;
    let op_id = read_u32(input, 16)?;
    let payload_len = read_u32(input, 20)? as usize;
    if request_id == 0 || op_id == 0 {
        return Err(CodecError::InvalidIdentity);
    }
    let expected = REQUEST_HEADER
        .checked_add(payload_len)
        .ok_or(CodecError::LengthOverflow)?;
    if payload_len > max_payload {
        return Err(CodecError::PayloadTooLarge);
    }
    if input.len() != expected {
        return Err(CodecError::InvalidLength);
    }
    Ok(RequestEnvelope {
        request_id,
        op_id,
        payload: input[REQUEST_HEADER..].to_vec(),
    })
}

/// Encodes a completion into the stable network-byte-order envelope.
///
/// # Errors
///
/// Rejects payloads that cannot be represented by the protocol length field.
pub fn encode_completion(completion: &CompletionEnvelope) -> Result<Vec<u8>, CodecError> {
    let payload_len =
        u32::try_from(completion.payload.len()).map_err(|_| CodecError::LengthOverflow)?;
    let mut output = Vec::with_capacity(COMPLETION_HEADER + completion.payload.len());
    output.extend_from_slice(&MAGIC);
    output.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    output.extend_from_slice(&COMPLETION_KIND.to_be_bytes());
    output.extend_from_slice(&completion.request_id.to_be_bytes());
    output.extend_from_slice(&completion.status.to_be_bytes());
    output.extend_from_slice(&0_u32.to_be_bytes());
    output.extend_from_slice(&payload_len.to_be_bytes());
    output.extend_from_slice(&completion.payload);
    Ok(output)
}

/// Decodes a completion using the same bounded rules as requests.
///
/// # Errors
///
/// Rejects malformed, oversized, unknown-version, wrong-kind, or zero-ID input.
pub fn decode_completion(
    input: &[u8],
    max_payload: usize,
) -> Result<CompletionEnvelope, CodecError> {
    validate_prefix(input, COMPLETION_HEADER, COMPLETION_KIND)?;
    let request_id = read_u64(input, 8)?;
    let status = read_u32(input, 16)?;
    let reserved = read_u32(input, 20)?;
    let payload_len = read_u32(input, 24)? as usize;
    if request_id == 0 {
        return Err(CodecError::InvalidIdentity);
    }
    if reserved != 0 {
        return Err(CodecError::UnknownField);
    }
    let expected = COMPLETION_HEADER
        .checked_add(payload_len)
        .ok_or(CodecError::LengthOverflow)?;
    if payload_len > max_payload {
        return Err(CodecError::PayloadTooLarge);
    }
    if input.len() != expected {
        return Err(CodecError::InvalidLength);
    }
    Ok(CompletionEnvelope {
        request_id,
        status,
        payload: input[COMPLETION_HEADER..].to_vec(),
    })
}

fn validate_prefix(input: &[u8], header: usize, kind: u16) -> Result<(), CodecError> {
    if input.len() < header {
        return Err(CodecError::Truncated);
    }
    if input[..4] != MAGIC {
        return Err(CodecError::InvalidMagic);
    }
    if read_u16(input, 4)? != PROTOCOL_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    if read_u16(input, 6)? != kind {
        return Err(CodecError::WrongEnvelopeKind);
    }
    Ok(())
}

fn read_u16(input: &[u8], offset: usize) -> Result<u16, CodecError> {
    let bytes: [u8; 2] = input
        .get(offset..offset + 2)
        .ok_or(CodecError::Truncated)?
        .try_into()
        .map_err(|_| CodecError::Truncated)?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(input: &[u8], offset: usize) -> Result<u32, CodecError> {
    let bytes: [u8; 4] = input
        .get(offset..offset + 4)
        .ok_or(CodecError::Truncated)?
        .try_into()
        .map_err(|_| CodecError::Truncated)?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64(input: &[u8], offset: usize) -> Result<u64, CodecError> {
    let bytes: [u8; 8] = input
        .get(offset..offset + 8)
        .ok_or(CodecError::Truncated)?
        .try_into()
        .map_err(|_| CodecError::Truncated)?;
    Ok(u64::from_be_bytes(bytes))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion,
    WrongEnvelopeKind,
    UnknownField,
    InvalidIdentity,
    InvalidLength,
    LengthOverflow,
    PayloadTooLarge,
}

#[cfg(test)]
mod tests {
    use super::{
        CodecError, CompletionEnvelope, RequestEnvelope, decode_completion, decode_request,
        encode_completion, encode_request,
    };

    #[test]
    // Verifies request envelopes round-trip without platform layout dependence.
    fn request_round_trip() {
        let request = RequestEnvelope {
            request_id: 1,
            op_id: 2,
            payload: vec![3, 4],
        };
        let encoded = encode_request(&request).expect("encoded");
        assert_eq!(decode_request(&encoded, 2), Ok(request));
    }

    #[test]
    // Verifies completion envelopes round-trip with explicit reserved bytes.
    fn completion_round_trip() {
        let completion = CompletionEnvelope {
            request_id: 1,
            status: 2,
            payload: vec![3],
        };
        assert_eq!(
            decode_completion(&encode_completion(&completion).expect("encoded"), 1),
            Ok(completion)
        );
    }

    #[test]
    // Verifies generated arbitrary byte envelopes either reject or canonicalize without panicking.
    fn generated_decoder_inputs_are_bounded_and_canonical() {
        let mut state = 0xa076_1d64_78bd_642f_u64;
        for length in 0..512 {
            let mut input = Vec::with_capacity(length);
            for _ in 0..length {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                input.push(state.to_le_bytes()[0]);
            }
            if let Ok(request) = decode_request(&input, 256) {
                assert_eq!(encode_request(&request).expect("canonical request"), input);
            }
            if let Ok(completion) = decode_completion(&input, 256) {
                assert_eq!(
                    encode_completion(&completion).expect("canonical completion"),
                    input
                );
            }
        }
    }

    #[test]
    // Verifies every short prefix fails safely rather than indexing out of bounds.
    fn all_truncated_request_prefixes_are_rejected() {
        let encoded = encode_request(&RequestEnvelope {
            request_id: 1,
            op_id: 1,
            payload: vec![1],
        })
        .expect("encoded");
        for length in 0..24 {
            assert_eq!(
                decode_request(&encoded[..length], 1),
                Err(CodecError::Truncated)
            );
        }
    }

    #[test]
    // Verifies a declared payload cannot force allocation beyond the configured cap.
    fn oversized_payload_is_rejected_before_copy() {
        let encoded = encode_request(&RequestEnvelope {
            request_id: 1,
            op_id: 1,
            payload: vec![1, 2],
        })
        .expect("encoded");
        assert_eq!(
            decode_request(&encoded, 1),
            Err(CodecError::PayloadTooLarge)
        );
    }

    #[test]
    // Verifies unknown protocol versions fail closed.
    fn unknown_version_is_rejected() {
        let mut encoded = encode_request(&RequestEnvelope {
            request_id: 1,
            op_id: 1,
            payload: Vec::new(),
        })
        .expect("encoded");
        encoded[5] = 2;
        assert_eq!(
            decode_request(&encoded, 1),
            Err(CodecError::UnsupportedVersion)
        );
    }

    #[test]
    // Verifies non-zero reserved fields are treated as unknown schema data.
    fn unknown_completion_field_is_rejected() {
        let mut encoded = encode_completion(&CompletionEnvelope {
            request_id: 1,
            status: 0,
            payload: Vec::new(),
        })
        .expect("encoded");
        encoded[23] = 1;
        assert_eq!(
            decode_completion(&encoded, 1),
            Err(CodecError::UnknownField)
        );
    }
}
