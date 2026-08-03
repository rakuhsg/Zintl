import ZintlAppkitSupportTypes

func zintlString(_ value: ZintlString) -> String {
  guard value.length > 0 else {
    return ""
  }
  guard let bytes = value.bytes else {
    preconditionFailure("ZintlString has a null buffer")
  }
  let buffer = UnsafeBufferPointer(start: bytes, count: Int(value.length))
  return String(decoding: buffer, as: UTF8.self)
}

func zintlOptionalString(_ value: ZintlOptionalString) -> String? {
  value.is_some ? zintlString(value.value) : nil
}

func withZintlString<Result>(
  _ value: String,
  _ body: (ZintlString) throws -> Result
) rethrows -> Result {
  let utf8 = Array(value.utf8)
  return try utf8.withUnsafeBufferPointer { buffer in
    try body(ZintlString(bytes: buffer.baseAddress, length: UInt(buffer.count)))
  }
}
