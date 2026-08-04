import RuntimeEmbed

try runtime.registerOp(
  name: "com.example.image.decode",
  version: 1,
  limits: try OpLimits(maxInputBytes: 4 << 20, maxOutputBytes: 16 << 20),
  permission: "com.example.image.decode"
) { context, input in
  try Task.checkCancellation()
  return try await decode(input, requestID: context.requestID)
}

