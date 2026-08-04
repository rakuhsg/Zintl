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

private let directoryKind: UInt32 = 3
private let fileKind: UInt32 = 4
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
    queue.sync {
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
    queue.async { [weak self] in self?.evaluate(evaluationID: evaluationID, source: source) }
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
    queue.async { [weak self] in
      self?.finishHostRequest(
        requestID: requestID, kind: kind, objectID: objectID, payload: payload)
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
    queue.sync {
      for promise in promises.values {
        _ = promise.reject.call(withArguments: [
          Self.errorObject("Runtime is shutting down", code: "ShuttingDown", context: context)
        ])
      }
      promises.removeAll()
      bridge = nil
      context = nil
    }
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
    typealias RequestDirectory = @convention(block) (NSString, Double, JSValue, JSValue) -> Void
    let requestDirectory: RequestDirectory = { [weak self] path, rights, resolve, reject in
      self?.submitDirectory(path: path as String, rights: rights, resolve: resolve, reject: reject)
    }
    typealias Open =
      @convention(block) (JSValue, NSString, Double, Double, JSValue, JSValue) -> Void
    let open: Open = { [weak self] receiver, path, rights, flags, resolve, reject in
      self?.submitOpen(receiver, path as String, rights, flags, resolve, reject)
    }
    typealias Read = @convention(block) (JSValue, Double, JSValue, JSValue) -> Void
    let read: Read = { [weak self] receiver, maximum, resolve, reject in
      self?.submitRead(receiver, maximum, resolve, reject)
    }
    typealias Write = @convention(block) (JSValue, JSValue, JSValue, JSValue) -> Void
    let write: Write = { [weak self] receiver, bytes, resolve, reject in
      self?.submitWrite(receiver, bytes, resolve, reject)
    }
    typealias ObjectOperation = @convention(block) (JSValue, JSValue, JSValue) -> Void
    let stat: ObjectOperation = { [weak self] receiver, resolve, reject in
      self?.submitObjectOperation(14, receiver, resolve, reject)
    }
    let close: ObjectOperation = { [weak self] receiver, resolve, reject in
      self?.submitObjectOperation(15, receiver, resolve, reject)
    }
    typealias Check = @convention(block) (JSValue, Double) -> Bool
    let check: Check = { [weak self] value, kind in
      self?.readObject(value, expectedKind: UInt32(kind)) != nil
    }
    typealias Console = @convention(block) (NSString) -> Void
    let console: Console = { [weak self] message in
      self?.emitConsole(message as String)
    }
    context.setObject(evaluationDone, forKeyedSubscript: "__zintlEvaluationDone" as NSString)
    context.setObject(invoke, forKeyedSubscript: "__zintlInvoke" as NSString)
    context.setObject(sleep, forKeyedSubscript: "__zintlSleep" as NSString)
    context.setObject(requestDirectory, forKeyedSubscript: "__zintlRequestDirectory" as NSString)
    context.setObject(open, forKeyedSubscript: "__zintlOpen" as NSString)
    context.setObject(read, forKeyedSubscript: "__zintlRead" as NSString)
    context.setObject(write, forKeyedSubscript: "__zintlWrite" as NSString)
    context.setObject(stat, forKeyedSubscript: "__zintlStat" as NSString)
    context.setObject(close, forKeyedSubscript: "__zintlClose" as NSString)
    context.setObject(check, forKeyedSubscript: "__zintlCheck" as NSString)
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

  private func submitDirectory(path: String, rights: Double, resolve: JSValue, reject: JSValue) {
    guard path.utf8.count <= 4096, let rights = exactUInt64(rights), rights > 0, rights & ~0x3F == 0
    else {
      rejectNow(reject, "Invalid directory request", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt64(rights, to: &payload)
    payload.append(Data(path.utf8))
    submitHost(kind: 10, payload: payload, resolve: resolve, reject: reject)
  }

  private func submitOpen(
    _ receiver: JSValue, _ path: String, _ rightsValue: Double, _ flagsValue: Double,
    _ resolve: JSValue, _ reject: JSValue
  ) {
    guard let directory = readObject(receiver, expectedKind: directoryKind),
      let rights = exactUInt64(rightsValue), let flags = exactUInt64(flagsValue), flags <= 3
    else {
      rejectNow(reject, "Invalid directory receiver", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt64(directory, to: &payload)
    appendUInt64(rights, to: &payload)
    payload.append(UInt8(flags))
    payload.append(Data(path.utf8))
    submitHost(kind: 11, payload: payload, resolve: resolve, reject: reject)
  }

  private func submitRead(
    _ receiver: JSValue, _ maximumValue: Double, _ resolve: JSValue, _ reject: JSValue
  ) {
    guard let file = readObject(receiver, expectedKind: fileKind),
      let maximum = exactUInt64(maximumValue)
    else {
      rejectNow(reject, "Invalid file receiver", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt64(file, to: &payload)
    appendUInt64(maximum, to: &payload)
    submitHost(kind: 12, payload: payload, resolve: resolve, reject: reject)
  }

  private func submitWrite(
    _ receiver: JSValue, _ value: JSValue, _ resolve: JSValue, _ reject: JSValue
  ) {
    guard let file = readObject(receiver, expectedKind: fileKind), let bytes = byteArray(value)
    else {
      rejectNow(reject, "Invalid file write", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt64(file, to: &payload)
    payload.append(bytes)
    submitHost(kind: 13, payload: payload, resolve: resolve, reject: reject)
  }

  private func submitObjectOperation(
    _ operation: UInt16, _ receiver: JSValue, _ resolve: JSValue, _ reject: JSValue
  ) {
    guard
      let object = readObject(receiver, expectedKind: directoryKind)
        ?? readObject(receiver, expectedKind: fileKind)
    else {
      rejectNow(reject, "Invalid resource receiver", code: "InvalidRequest")
      return
    }
    var payload = Data()
    appendUInt64(object, to: &payload)
    submitHost(kind: operation, payload: payload, resolve: resolve, reject: reject)
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

  private func finishHostRequest(requestID: UInt64, kind: UInt32, objectID: UInt64, payload: Data) {
    stateLock.lock()
    pendingHostIDs.remove(requestID)
    stateLock.unlock()
    guard let promise = promises.removeValue(forKey: requestID) else { return }
    switch kind {
    case 0:
      _ = promise.resolve.call(withArguments: [])
    case 1:
      _ = promise.resolve.call(withArguments: [Array(payload)])
    case 2, 3:
      let hostKind = kind == 2 ? directoryKind : fileKind
      guard let context,
        let object = rtjsc_host_object_make(
          context.jsGlobalContextRef, runtimeID, objectID, hostKind),
        let value = JSValue(jsValueRef: object, in: context), let bridge
      else {
        rejectNow(promise.reject, "Host object creation failed", code: "Backend")
        return
      }
      let decorated = bridge.invokeMethod("decorate", withArguments: [value, Double(hostKind)])
      _ = promise.resolve.call(withArguments: [decorated as Any])
    default:
      let code = String(data: payload, encoding: .utf8) ?? "OperationFailed"
      rejectNow(promise.reject, "Host operation failed", code: code)
    }
  }

  private func readObject(_ value: JSValue, expectedKind: UInt32) -> UInt64? {
    guard let context else { return nil }
    var objectID: UInt64 = 0
    guard
      rtjsc_host_object_read(
        context.jsGlobalContextRef, value.jsValueRef, runtimeID, expectedKind, &objectID) == 0
    else { return nil }
    return objectID
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

  private let queue = DispatchQueue(label: "dev.zintl.runtime-jsc-ffi")
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
      const directoryNative = globalThis.__zintlRequestDirectory;
      const openNative = globalThis.__zintlOpen;
      const readNative = globalThis.__zintlRead;
      const writeNative = globalThis.__zintlWrite;
      const statNative = globalThis.__zintlStat;
      const closeNative = globalThis.__zintlClose;
      const checkNative = globalThis.__zintlCheck;
      const consoleNative = globalThis.__zintlConsole;
      for (const name of ["__zintlEvaluationDone", "__zintlInvoke", "__zintlSleep",
        "__zintlRequestDirectory", "__zintlOpen", "__zintlRead", "__zintlWrite",
        "__zintlStat", "__zintlClose", "__zintlCheck", "__zintlConsole"]) delete globalThis[name];
      const error = (message, code) => Object.assign(new Error(message), { code });
      const rights = Object.freeze({read:1, write:2, create:4, metadata:8, enumerate:16, truncate:32});
      const encodeRights = (options, includeMutationRights = false) => {
        if (!options || typeof options !== "object") throw error("Invalid rights", "InvalidRequest");
        let bits = 0;
        for (const key of Object.keys(options)) {
          if (key === "create" || key === "truncate") {
            if (typeof options[key] !== "boolean") throw error("Invalid rights", "InvalidRequest");
            if (includeMutationRights && options[key]) bits |= rights[key];
            continue;
          }
          if (!(key in rights) || typeof options[key] !== "boolean") throw error("Invalid rights", "InvalidRequest");
          if (options[key]) bits |= rights[key];
        }
        if (!bits) throw error("Empty rights", "InvalidRequest");
        return bits;
      };
      const invoke = (name, input = new Uint8Array()) => new Promise((resolve, reject) => {
        if (typeof name !== "string" || !(input instanceof Uint8Array)) return reject(error("Invalid input", "InvalidRequest"));
        invokeNative(name, Array.from(input), resolve, reject);
      }).then(bytes => new Uint8Array(bytes));
      const sleep = ms => new Promise((resolve, reject) => sleepNative(ms, resolve, reject));
      const requestDirectory = (path, options) => new Promise((resolve, reject) => {
        try { directoryNative(path, encodeRights(options, true), resolve, reject); } catch (e) { reject(e); }
      });
      const decorate = (object, kind) => {
        const check = self => checkNative(self, kind);
        if (kind === 3) Object.defineProperties(object, {
          openRelative: { value(path, options) { return new Promise((resolve, reject) => {
            if (!check(this)) return reject(error("Invalid receiver", "InvalidRequest"));
            try { const flags = (options?.create ? 1 : 0) | (options?.truncate ? 2 : 0);
              openNative(this, path, encodeRights(options), flags, resolve, reject); } catch (e) { reject(e); }
          }); }},
          readRelative: { value(path, options = {}) {
            return this.openRelative(path, {read:true}).then(async file => {
              try { return await file.read(options); } finally { await file.close(); }
            });
          }},
          writeRelative: { value(path, bytes, options = {}) {
            if (!(bytes instanceof Uint8Array)) return Promise.reject(error("Invalid bytes", "InvalidRequest"));
            return this.openRelative(path, {write:true, create:options.create === true,
              truncate:options.truncate === true}).then(async file => {
              try { await file.write(bytes); } finally { await file.close(); }
            });
          }},
          statRelative: { value(path) {
            return this.openRelative(path, {metadata:true}).then(async file => {
              try { return await file.stat(); } finally { await file.close(); }
            });
          }},
          close: { value() { return new Promise((resolve, reject) => closeNative(this, resolve, reject)); }}
        });
        if (kind === 4) Object.defineProperties(object, {
          read: { value(options = {}) { return new Promise((resolve, reject) => readNative(this, options.maxBytes ?? 65536, resolve, reject)).then(x => new Uint8Array(x)); }},
          write: { value(bytes) { return new Promise((resolve, reject) => writeNative(this, Array.from(bytes), resolve, reject)); }},
          stat: { value() { return new Promise((resolve, reject) => statNative(this, resolve, reject)).then(bytes => {
            let size = 0; for (let i = 1; i < 9; i++) size = size * 256 + bytes[i];
            return Object.freeze({kind: bytes[0] === 1 ? "file" : "directory", size});
          }); }},
          close: { value() { return new Promise((resolve, reject) => closeNative(this, resolve, reject)); }}
        });
        return Object.freeze(object);
      };
      Object.defineProperty(globalThis, "Zintl", {value:Object.freeze({invoke, sleep, requestDirectory}), configurable:false});
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
      return Object.freeze({evaluate, decorate});
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
