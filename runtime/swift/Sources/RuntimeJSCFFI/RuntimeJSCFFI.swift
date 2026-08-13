import Foundation
import JavaScriptCore
import RuntimeJSCShim

private let ok: UInt32 = 0
private let empty: UInt32 = 1
private let bufferTooSmall: UInt32 = 2
private let invalidArgument: UInt32 = 3
private let invalidState: UInt32 = 4
private let quotaExceeded: UInt32 = 5
private let backendError: UInt32 = 255

private let maximumSafeInteger: UInt64 = 9_007_199_254_740_991

public typealias NotifyFunction = @convention(c) (UnsafeMutableRawPointer?) -> Void

private struct PendingPromise {
  let resolve: JSValue
  let reject: JSValue
}

private final class JSCFFIEngine: @unchecked Sendable {
  init(
    maximumEvaluations: Int,
    maximumHostRequests: Int,
    maximumSourceBytes: Int,
    maximumEventBytes: Int,
    notify: NotifyFunction?,
    userData: UnsafeMutableRawPointer?
  ) {
    self.maximumEvaluations = maximumEvaluations
    self.maximumHostRequests = maximumHostRequests
    self.maximumSourceBytes = maximumSourceBytes
    self.maximumEventBytes = maximumEventBytes
    self.notify = notify
    self.userData = userData
    runtimeID = UInt64.random(in: 1...UInt64.max)
  }

  func start() -> UInt32 {
    guard context == nil, !shuttingDown else { return invalidState }
    guard let context = JSContext() else { return backendError }
    self.context = context
    installCallbacks(in: context)
    guard let bridge = context.evaluateScript(Self.bootstrap), context.exception == nil else {
      context.exception = nil
      self.context = nil
      return backendError
    }
    self.bridge = bridge
    return ok
  }

  func submit(evaluationID: UInt64, source: Data) -> UInt32 {
    guard evaluationID != 0, evaluationID <= maximumSafeInteger,
      source.count <= maximumSourceBytes,
      let source = String(data: source, encoding: .utf8)
    else { return invalidArgument }
    stateLock.lock()
    guard !shuttingDown else {
      stateLock.unlock()
      return invalidState
    }
    eventLock.lock()
    let terminalCapacityAvailable = terminalEvents.count < maximumEvaluations
    eventLock.unlock()
    guard terminalCapacityAvailable else {
      stateLock.unlock()
      return quotaExceeded
    }
    guard pendingEvaluations.insert(evaluationID).inserted,
      pendingEvaluations.count <= maximumEvaluations
    else {
      pendingEvaluations.remove(evaluationID)
      stateLock.unlock()
      return quotaExceeded
    }
    stateLock.unlock()
    evaluate(evaluationID: evaluationID, source: source)
    return ok
  }

  func nextEvent(output: UnsafeMutablePointer<UInt8>?, capacity: Int, required: inout Int) -> UInt32
  {
    eventLock.lock()
    defer { eventLock.unlock() }
    let terminal = events.isEmpty ? terminalEvents.first : nil
    guard let event = events.first ?? terminal else {
      required = 0
      return empty
    }
    required = event.count
    guard capacity >= event.count, let output else { return bufferTooSmall }
    event.copyBytes(to: output, count: event.count)
    if terminal != nil {
      terminalEvents.removeFirst()
    } else {
      events.removeFirst()
      queuedEventBytes -= event.count
    }
    return ok
  }

  func complete(
    requestID: UInt64,
    kind: UInt32,
    objectID: UInt64,
    payload: Data
  ) -> UInt32 {
    stateLock.lock()
    let canAccept = !shuttingDown && pendingHostIDs.contains(requestID)
    stateLock.unlock()
    guard canAccept else { return invalidArgument }
    finishHostRequest(requestID: requestID, kind: kind, objectID: objectID, payload: payload)
    return ok
  }

  func microtaskCheckpoint() -> UInt32 {
    guard let context, !shuttingDown else { return invalidState }
    // JavaScriptCore drains Promise jobs at the end of a host entry. A no-op
    // entry gives jobs queued by native Promise settlement an explicit checkpoint.
    _ = context.evaluateScript("void 0")
    if context.exception != nil {
      context.exception = nil
      return backendError
    }
    return ok
  }

  func cancel(evaluationID: UInt64) -> UInt32 {
    stateLock.lock()
    guard pendingEvaluations.contains(evaluationID) else {
      stateLock.unlock()
      return invalidArgument
    }
    cancelledEvaluations.insert(evaluationID)
    stateLock.unlock()
    return ok
  }

  func shutdown() -> UInt32 {
    stateLock.lock()
    if shuttingDown {
      stateLock.unlock()
      return ok
    }
    shuttingDown = true
    stateLock.unlock()
    for promise in promises.values {
      _ = promise.reject.call(withArguments: [
        Self.errorObject("Runtime is shutting down", code: "ShuttingDown", context: context)
      ])
    }
    promises.removeAll()
    bridge = nil
    context = nil
    stateLock.lock()
    pendingHostIDs.removeAll()
    pendingEvaluations.removeAll()
    cancelledEvaluations.removeAll()
    stateLock.unlock()
    return ok
  }

  private func installCallbacks(in context: JSContext) {
    typealias EvaluationDone = @convention(block) (Double, Bool, NSString) -> Void
    let evaluationDone: EvaluationDone = { [weak self] identifier, succeeded, value in
      self?.finishEvaluation(
        evaluationID: UInt64(identifier), succeeded: succeeded, value: value as String)
    }
    typealias Invoke = @convention(block) (NSString, JSValue, JSValue, JSValue) -> Void
    let invoke: Invoke = { [weak self] name, bytes, resolve, reject in
      self?.submitInvoke(name: name as String, bytes: bytes, resolve: resolve, reject: reject)
    }
    typealias Sleep = @convention(block) (Double, JSValue, JSValue) -> Void
    let sleep: Sleep = { [weak self] milliseconds, resolve, reject in
      self?.submitSleep(milliseconds: milliseconds, resolve: resolve, reject: reject)
    }
    typealias ReadFile = @convention(block) (NSString, Double, JSValue, JSValue) -> Void
    let readFile: ReadFile = { [weak self] url, maximum, resolve, reject in
      self?.submitMountRead(url: url as String, maximum: maximum, resolve: resolve, reject: reject)
    }
    typealias MountOperation =
      @convention(block) (
        Double, NSString, JSValue, JSValue, JSValue
      ) -> Void
    let mountOperation: MountOperation = { [weak self] operation, first, second, resolve, reject in
      self?.submitMountOperation(
        operation: operation, first: first as String, second: second,
        resolve: resolve, reject: reject)
    }
    typealias Console = @convention(block) (NSString) -> Void
    let console: Console = { [weak self] message in
      self?.emitConsole(message as String)
    }
    context.setObject(evaluationDone, forKeyedSubscript: "__zintlEvaluationDone" as NSString)
    context.setObject(invoke, forKeyedSubscript: "__zintlInvoke" as NSString)
    context.setObject(sleep, forKeyedSubscript: "__zintlSleep" as NSString)
    context.setObject(readFile, forKeyedSubscript: "__zintlReadFile" as NSString)
    context.setObject(mountOperation, forKeyedSubscript: "__zintlMountOperation" as NSString)
    context.setObject(console, forKeyedSubscript: "__zintlConsole" as NSString)
  }

  private func emitConsole(_ message: String) {
    let bytes = Data(message.utf8)
    guard bytes.count <= maximumEventBytes, let length = UInt32(exactly: bytes.count) else {
      return
    }
    var event = Data()
    event.append(contentsOf: [0x5A, 0x4A, 0x45, 0x31])
    appendUInt16(1, to: &event)
    appendUInt16(5, to: &event)
    appendUInt64(0, to: &event)
    appendUInt32(length, to: &event)
    event.append(bytes)
    enqueue(event)
  }

  private func evaluate(evaluationID: UInt64, source: String) {
    guard let bridge, !shuttingDown else {
      finishEvaluation(
        evaluationID: evaluationID, succeeded: false, value: "Error: Runtime is shutting down")
      return
    }
    _ = bridge.invokeMethod("evaluate", withArguments: [Double(evaluationID), source])
    if let exception = context?.exception {
      context?.exception = nil
      finishEvaluation(
        evaluationID: evaluationID, succeeded: false,
        value: "Error: \(exception.toString() ?? "JavaScript evaluation failed")")
    }
  }

  private func finishEvaluation(evaluationID: UInt64, succeeded: Bool, value: String) {
    stateLock.lock()
    let wasPending = pendingEvaluations.remove(evaluationID) != nil
    let cancelled = cancelledEvaluations.remove(evaluationID) != nil
    stateLock.unlock()
    guard wasPending else { return }
    var payload = Data()
    payload.append(contentsOf: [0x5A, 0x4A, 0x45, 0x31])
    appendUInt16(1, to: &payload)
    appendUInt16(cancelled ? 3 : (succeeded ? 1 : 2), to: &payload)
    appendUInt64(evaluationID, to: &payload)
    let bytes = Data(value.utf8)
    appendUInt32(UInt32(bytes.count), to: &payload)
    payload.append(bytes)
    if !enqueue(payload) {
      var fallback = Data()
      fallback.append(contentsOf: [0x5A, 0x4A, 0x45, 0x31])
      appendUInt16(1, to: &fallback)
      appendUInt16(2, to: &fallback)
      appendUInt64(evaluationID, to: &fallback)
      let message = Data("QuotaExceeded: Engine event queue exhausted".utf8)
      appendUInt32(UInt32(message.count), to: &fallback)
      fallback.append(message)
      eventLock.lock()
      if terminalEvents.count < maximumEvaluations { terminalEvents.append(fallback) }
      eventLock.unlock()
      notify?(userData)
    }
  }

  private func submitInvoke(name: String, bytes: JSValue, resolve: JSValue, reject: JSValue) {
    guard let input = byteArray(bytes), name.utf8.count <= UInt16.max else {
      rejectNow(reject, "Invalid operation input", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt32(1, to: &payload)
    appendUInt16(UInt16(name.utf8.count), to: &payload)
    payload.append(Data(name.utf8))
    payload.append(input)
    submitHost(kind: 1, payload: payload, resolve: resolve, reject: reject)
  }

  private func submitSleep(milliseconds: Double, resolve: JSValue, reject: JSValue) {
    guard milliseconds.isFinite, milliseconds >= 0, milliseconds <= 86_400_000,
      milliseconds.rounded(.down) == milliseconds
    else {
      rejectNow(reject, "Invalid timer delay", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt64(UInt64(milliseconds) * 1_000_000, to: &payload)
    submitHost(kind: 2, payload: payload, resolve: resolve, reject: reject)
  }

  private func submitMountRead(
    url: String, maximum: Double, resolve: JSValue, reject: JSValue
  ) {
    guard url.utf8.count <= 4096, let maximum = exactUInt64(maximum), maximum > 0
    else {
      rejectNow(reject, "Invalid mount read request", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt64(maximum, to: &payload)
    payload.append(Data(url.utf8))
    submitHost(kind: 10, payload: payload, resolve: resolve, reject: reject)
  }

  private func submitMountOperation(
    operation: Double, first: String, second: JSValue, resolve: JSValue, reject: JSValue
  ) {
    guard let kind = exactUInt64(operation), (11...15).contains(kind), first.utf8.count <= 4096
    else {
      rejectNow(reject, "Invalid mount operation", code: "InvalidRequest")
      return
    }
    var payload = Data()
    switch kind {
    case 11:
      guard let length = UInt16(exactly: first.utf8.count), let bytes = byteArray(second) else {
        rejectNow(reject, "Invalid mount write", code: "InvalidRequest")
        return
      }
      appendUInt16(length, to: &payload)
      payload.append(Data(first.utf8))
      payload.append(bytes)
    case 12...14:
      payload.append(Data(first.utf8))
    case 15:
      guard second.isString, let destination = second.toString(),
        destination.utf8.count <= 4096, let length = UInt16(exactly: first.utf8.count)
      else {
        rejectNow(reject, "Invalid mount rename", code: "InvalidRequest")
        return
      }
      appendUInt16(length, to: &payload)
      payload.append(Data(first.utf8))
      payload.append(Data(destination.utf8))
    default:
      rejectNow(reject, "Invalid mount operation", code: "InvalidRequest")
      return
    }
    submitHost(kind: UInt16(kind), payload: payload, resolve: resolve, reject: reject)
  }

  private func submitHost(kind: UInt16, payload body: Data, resolve: JSValue, reject: JSValue) {
    stateLock.lock()
    guard !shuttingDown, pendingHostIDs.count < maximumHostRequests, nextHostID < UInt64.max else {
      stateLock.unlock()
      rejectNow(reject, "Host request limit exceeded", code: "QuotaExceeded")
      return
    }
    let requestID = nextHostID
    nextHostID += 1
    pendingHostIDs.insert(requestID)
    stateLock.unlock()
    promises[requestID] = PendingPromise(resolve: resolve, reject: reject)
    var event = Data()
    event.append(contentsOf: [0x5A, 0x4A, 0x45, 0x31])
    appendUInt16(1, to: &event)
    appendUInt16(4, to: &event)
    appendUInt64(requestID, to: &event)
    appendUInt16(kind, to: &event)
    appendUInt32(UInt32(body.count), to: &event)
    event.append(body)
    if !enqueue(event) {
      stateLock.lock()
      pendingHostIDs.remove(requestID)
      stateLock.unlock()
      promises.removeValue(forKey: requestID)
      rejectNow(reject, "Engine event queue exhausted", code: "QuotaExceeded")
    }
  }

  private func finishHostRequest(requestID: UInt64, kind: UInt32, objectID _: UInt64, payload: Data)
  {
    stateLock.lock()
    pendingHostIDs.remove(requestID)
    stateLock.unlock()
    guard let promise = promises.removeValue(forKey: requestID) else { return }
    switch kind {
    case 0:
      _ = promise.resolve.call(withArguments: [])
    case 1:
      _ = promise.resolve.call(withArguments: [Array(payload)])
    default:
      let fallbackCode = "OperationFailed"
      let fallbackMessage = "Host operation failed"
      guard payload.count >= 2 else {
        rejectNow(promise.reject, fallbackMessage, code: fallbackCode)
        return
      }
      let codeLength =
        (Int(payload[payload.startIndex]) << 8)
        | Int(payload[payload.index(after: payload.startIndex)])
      guard payload.count >= 2 + codeLength else {
        rejectNow(promise.reject, fallbackMessage, code: fallbackCode)
        return
      }
      let codeStart = payload.index(payload.startIndex, offsetBy: 2)
      let messageStart = payload.index(codeStart, offsetBy: codeLength)
      let code = String(data: payload[codeStart..<messageStart], encoding: .utf8) ?? fallbackCode
      let message = String(data: payload[messageStart...], encoding: .utf8) ?? fallbackMessage
      rejectNow(promise.reject, message, code: code)
    }
  }

  private func byteArray(_ value: JSValue) -> Data? {
    guard let array = value.toArray(), array.count <= maximumEventBytes else { return nil }
    var data = Data(capacity: array.count)
    for item in array {
      guard let number = item as? NSNumber else { return nil }
      let value = number.intValue
      guard (0...255).contains(value) else { return nil }
      data.append(UInt8(value))
    }
    return data
  }

  @discardableResult
  private func enqueue(_ event: Data) -> Bool {
    eventLock.lock()
    let accepted =
      terminalEvents.isEmpty
      && event.count <= maximumEventBytes
      && queuedEventBytes <= maximumEventBytes - event.count
    if accepted {
      events.append(event)
      queuedEventBytes += event.count
    }
    eventLock.unlock()
    if accepted { notify?(userData) }
    return accepted
  }

  private func rejectNow(_ reject: JSValue, _ message: String, code: String) {
    _ = reject.call(withArguments: [Self.errorObject(message, code: code, context: context)])
  }

  private static func errorObject(_ message: String, code: String, context: JSContext?) -> Any {
    guard let context, let error = JSValue(newErrorFromMessage: message, in: context) else {
      return message
    }
    error.setValue(code, forProperty: "code")
    return error
  }

  private func exactUInt64(_ value: Double) -> UInt64? {
    guard value.isFinite, value >= 0, value <= 9_007_199_254_740_991,
      value.rounded(.down) == value
    else { return nil }
    return UInt64(value)
  }

  private func appendUInt16(_ value: UInt16, to data: inout Data) {
    data.append(UInt8(value >> 8))
    data.append(UInt8(value & 0xFF))
  }

  private func appendUInt32(_ value: UInt32, to data: inout Data) {
    for shift in stride(from: 24, through: 0, by: -8) {
      data.append(UInt8((value >> UInt32(shift)) & 0xFF))
    }
  }

  private func appendUInt64(_ value: UInt64, to data: inout Data) {
    for shift in stride(from: 56, through: 0, by: -8) {
      data.append(UInt8((value >> UInt64(shift)) & 0xFF))
    }
  }

  private let eventLock = NSLock()
  private let stateLock = NSLock()
  private let maximumEvaluations: Int
  private let maximumHostRequests: Int
  private let maximumSourceBytes: Int
  private let maximumEventBytes: Int
  private let notify: NotifyFunction?
  private let userData: UnsafeMutableRawPointer?
  private let runtimeID: UInt64
  private var context: JSContext?
  private var bridge: JSValue?
  private var promises: [UInt64: PendingPromise] = [:]
  private var pendingHostIDs = Set<UInt64>()
  private var pendingEvaluations = Set<UInt64>()
  private var cancelledEvaluations = Set<UInt64>()
  private var nextHostID: UInt64 = 1
  private var shuttingDown = false
  private var events: [Data] = []
  private var terminalEvents: [Data] = []
  private var queuedEventBytes = 0

  private static let bootstrap = #"""
    (() => {
      "use strict";
      const evaluationDone = globalThis.__zintlEvaluationDone;
      const invokeNative = globalThis.__zintlInvoke;
      const sleepNative = globalThis.__zintlSleep;
      const readFileNative = globalThis.__zintlReadFile;
      const mountOperationNative = globalThis.__zintlMountOperation;
      const consoleNative = globalThis.__zintlConsole;
      for (const name of ["__zintlEvaluationDone", "__zintlInvoke", "__zintlSleep",
        "__zintlReadFile", "__zintlMountOperation", "__zintlConsole"]) delete globalThis[name];
      const error = (message, code) => Object.assign(new Error(message), { code });
      const invoke = (name, input = new Uint8Array()) => new Promise((resolve, reject) => {
        if (typeof name !== "string" || !(input instanceof Uint8Array)) return reject(error("Invalid input", "InvalidRequest"));
        invokeNative(name, Array.from(input), resolve, reject);
      }).then(bytes => new Uint8Array(bytes));
      const sleep = ms => new Promise((resolve, reject) => sleepNative(ms, resolve, reject));
      const decodeUtf8 = bytes => {
        let output = "";
        let codePoints = [];
        const append = value => {
          codePoints.push(value);
          if (codePoints.length === 1024) {
            output += String.fromCodePoint(...codePoints);
            codePoints = [];
          }
        };
        const continuation = value => (value & 0xc0) === 0x80;
        for (let index = 0; index < bytes.length;) {
          const first = bytes[index++];
          if (first <= 0x7f) { append(first); continue; }
          if (first >= 0xc2 && first <= 0xdf && index < bytes.length) {
            const second = bytes[index++];
            if (continuation(second)) { append(((first & 0x1f) << 6) | (second & 0x3f)); continue; }
          } else if (first >= 0xe0 && first <= 0xef && index + 1 < bytes.length) {
            const second = bytes[index++], third = bytes[index++];
            const validSecond = continuation(second) && (first !== 0xe0 || second >= 0xa0)
              && (first !== 0xed || second <= 0x9f);
            if (validSecond && continuation(third)) {
              append(((first & 0x0f) << 12) | ((second & 0x3f) << 6) | (third & 0x3f)); continue;
            }
          } else if (first >= 0xf0 && first <= 0xf4 && index + 2 < bytes.length) {
            const second = bytes[index++], third = bytes[index++], fourth = bytes[index++];
            const validSecond = continuation(second) && (first !== 0xf0 || second >= 0x90)
              && (first !== 0xf4 || second <= 0x8f);
            if (validSecond && continuation(third) && continuation(fourth)) {
              append(((first & 0x07) << 18) | ((second & 0x3f) << 12)
                | ((third & 0x3f) << 6) | (fourth & 0x3f)); continue;
            }
          }
          throw error("Invalid UTF-8", "InvalidData");
        }
        if (codePoints.length) output += String.fromCodePoint(...codePoints);
        return output;
      };
      const encodeUtf8 = value => {
        const output = [];
        for (const character of value) {
          const point = character.codePointAt(0);
          if (point <= 0x7f) output.push(point);
          else if (point <= 0x7ff) output.push(0xc0 | (point >> 6), 0x80 | (point & 0x3f));
          else if (point <= 0xffff) output.push(0xe0 | (point >> 12),
            0x80 | ((point >> 6) & 0x3f), 0x80 | (point & 0x3f));
          else output.push(0xf0 | (point >> 18), 0x80 | ((point >> 12) & 0x3f),
            0x80 | ((point >> 6) & 0x3f), 0x80 | (point & 0x3f));
        }
        return new Uint8Array(output);
      };
      const readFile = (url, encoding) => new Promise((resolve, reject) => {
        if (typeof url !== "string" || (encoding !== undefined && encoding !== "utf8")) {
          return reject(error("Invalid mount URI or encoding", "InvalidRequest"));
        }
        readFileNative(url, 4 * 1024 * 1024, resolve, reject);
      }).then(bytes => {
        const value = new Uint8Array(bytes);
        return encoding === "utf8" ? decodeUtf8(value) : value;
      });
      const mutateMount = (kind, first, second = null) => new Promise((resolve, reject) => {
        if (typeof first !== "string") return reject(error("Invalid mount URI", "InvalidRequest"));
        mountOperationNative(kind, first, second, resolve, reject);
      });
      const writeFile = (url, data) => {
        let bytes;
        if (typeof data === "string") bytes = encodeUtf8(data);
        else if (data instanceof Uint8Array) bytes = data;
        else return Promise.reject(error("Expected string or Uint8Array", "InvalidRequest"));
        return mutateMount(11, url, Array.from(bytes));
      };
      const mkdir = url => mutateMount(12, url);
      const removeFile = url => mutateMount(13, url);
      const removeDirectory = url => mutateMount(14, url);
      const rename = (from, to) => {
        if (typeof to !== "string") return Promise.reject(error("Invalid mount URI", "InvalidRequest"));
        return mutateMount(15, from, to);
      };
      Object.defineProperty(globalThis, "Zintl", {
        value:Object.freeze({
          invoke, sleep, readFile, writeFile, mkdir, removeFile, removeDirectory, rename
        }), configurable:false
      });
      const formatConsoleValue = value => {
        if (typeof value === "string") return value;
        try { const encoded = JSON.stringify(value); if (encoded !== undefined) return encoded; } catch (_) {}
        try { return String(value); } catch (_) { return "<unprintable>"; }
      };
      const emitConsole = (...values) => consoleNative(values.map(formatConsoleValue).join(" "));
      Object.defineProperty(globalThis, "console", {value:Object.freeze({
        debug:emitConsole, log:emitConsole, info:emitConsole, warn:emitConsole, error:emitConsole
      }), configurable:false});
      const encode = value => JSON.stringify(value instanceof Uint8Array
        ? {type:"bytes", value:Array.from(value)} : {type:"value", value:value === undefined ? null : value});
      const evaluate = (id, source) => {
        let result; try { result = (0, eval)(source); } catch (e) { evaluationDone(id, false, `${e.name}: ${e.message}`); return; }
        Promise.resolve(result).then(value => evaluationDone(id, true, encode(value)),
          e => evaluationDone(id, false, `${e?.name ?? "Error"}: ${e?.message ?? e}`));
      };
      return Object.freeze({evaluate});
    })()
    """#
}

private func engine(_ pointer: UnsafeMutableRawPointer?) -> JSCFFIEngine? {
  guard let pointer else { return nil }
  return Unmanaged<JSCFFIEngine>.fromOpaque(pointer).takeUnretainedValue()
}

@_cdecl("zjsc_engine_new")
public func zjscEngineNew(
  _ maximumEvaluations: UInt32, _ maximumHostRequests: UInt32,
  _ maximumSourceBytes: UInt32, _ maximumEventBytes: UInt32,
  _ notify: NotifyFunction?, _ userData: UnsafeMutableRawPointer?,
  _ output: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> UInt32 {
  guard maximumEvaluations > 0, maximumHostRequests > 0, maximumSourceBytes > 0,
    maximumEventBytes > 0, let output
  else { return invalidArgument }
  let value = JSCFFIEngine(
    maximumEvaluations: Int(maximumEvaluations), maximumHostRequests: Int(maximumHostRequests),
    maximumSourceBytes: Int(maximumSourceBytes), maximumEventBytes: Int(maximumEventBytes),
    notify: notify, userData: userData)
  output.pointee = Unmanaged.passRetained(value).toOpaque()
  return ok
}

@_cdecl("zjsc_engine_start")
public func zjscEngineStart(_ pointer: UnsafeMutableRawPointer?) -> UInt32 {
  engine(pointer)?.start() ?? invalidArgument
}

@_cdecl("zjsc_engine_submit")
public func zjscEngineSubmit(
  _ pointer: UnsafeMutableRawPointer?, _ evaluationID: UInt64,
  _ source: UnsafePointer<UInt8>?, _ length: Int
) -> UInt32 {
  guard let engine = engine(pointer), length >= 0, length == 0 || source != nil else {
    return invalidArgument
  }
  let data = length == 0 ? Data() : Data(bytes: source!, count: length)
  return engine.submit(evaluationID: evaluationID, source: data)
}

@_cdecl("zjsc_engine_next_event")
public func zjscEngineNextEvent(
  _ pointer: UnsafeMutableRawPointer?, _ output: UnsafeMutablePointer<UInt8>?,
  _ capacity: Int, _ required: UnsafeMutablePointer<Int>?
) -> UInt32 {
  guard let engine = engine(pointer), capacity >= 0, let required else { return invalidArgument }
  var size = 0
  let status = engine.nextEvent(output: output, capacity: capacity, required: &size)
  required.pointee = size
  return status
}

@_cdecl("zjsc_engine_complete")
public func zjscEngineComplete(
  _ pointer: UnsafeMutableRawPointer?, _ requestID: UInt64, _ kind: UInt32,
  _ objectID: UInt64, _ payload: UnsafePointer<UInt8>?, _ length: Int
) -> UInt32 {
  guard let engine = engine(pointer), requestID != 0, length >= 0, length == 0 || payload != nil
  else { return invalidArgument }
  let data = length == 0 ? Data() : Data(bytes: payload!, count: length)
  return engine.complete(requestID: requestID, kind: kind, objectID: objectID, payload: data)
}

@_cdecl("zjsc_engine_cancel")
public func zjscEngineCancel(_ pointer: UnsafeMutableRawPointer?, _ evaluationID: UInt64) -> UInt32
{
  engine(pointer)?.cancel(evaluationID: evaluationID) ?? invalidArgument
}

@_cdecl("zjsc_engine_microtask_checkpoint")
public func zjscEngineMicrotaskCheckpoint(_ pointer: UnsafeMutableRawPointer?) -> UInt32 {
  engine(pointer)?.microtaskCheckpoint() ?? invalidArgument
}

@_cdecl("zjsc_engine_shutdown")
public func zjscEngineShutdown(_ pointer: UnsafeMutableRawPointer?) -> UInt32 {
  engine(pointer)?.shutdown() ?? invalidArgument
}

@_cdecl("zjsc_engine_free")
public func zjscEngineFree(_ pointer: UnsafeMutableRawPointer?) {
  guard let pointer else { return }
  let engine = Unmanaged<JSCFFIEngine>.fromOpaque(pointer).takeRetainedValue()
  _ = engine.shutdown()
}
