(() => {
  "use strict";
  const evaluationDone = globalThis.__zintlEvaluationDone;
  const invokeNative = globalThis.__zintlInvoke;
  const sleepNative = globalThis.__zintlSleep;
  const readFileNative = globalThis.__zintlReadFile;
  const mountOperationNative = globalThis.__zintlMountOperation;
  const consoleNative = globalThis.__zintlConsole;
  for (const name of [
    "__zintlEvaluationDone",
    "__zintlInvoke",
    "__zintlSleep",
    "__zintlReadFile",
    "__zintlMountOperation",
    "__zintlConsole",
  ]) delete globalThis[name];

  const error = (message, code) => Object.assign(new Error(message), { code });
  const invoke = (name, input = new Uint8Array()) =>
    new Promise((resolve, reject) => {
      if (typeof name !== "string" || !(input instanceof Uint8Array)) {
        return reject(error("Invalid input", "InvalidRequest"));
      }
      invokeNative(name, Array.from(input), resolve, reject);
    }).then((bytes) => new Uint8Array(bytes));
  const sleep = (ms) =>
    new Promise((resolve, reject) => sleepNative(ms, resolve, reject));

  const decodeUtf8 = (bytes) => {
    let output = "";
    let codePoints = [];
    const append = (value) => {
      codePoints.push(value);
      if (codePoints.length === 1024) {
        output += String.fromCodePoint(...codePoints);
        codePoints = [];
      }
    };
    const continuation = (value) => (value & 0xc0) === 0x80;
    for (let index = 0; index < bytes.length; ) {
      const first = bytes[index++];
      if (first <= 0x7f) {
        append(first);
        continue;
      }
      if (first >= 0xc2 && first <= 0xdf && index < bytes.length) {
        const second = bytes[index++];
        if (continuation(second)) {
          append(((first & 0x1f) << 6) | (second & 0x3f));
          continue;
        }
      } else if (first >= 0xe0 && first <= 0xef && index + 1 < bytes.length) {
        const second = bytes[index++];
        const third = bytes[index++];
        const validSecond =
          continuation(second) &&
          (first !== 0xe0 || second >= 0xa0) &&
          (first !== 0xed || second <= 0x9f);
        if (validSecond && continuation(third)) {
          append(((first & 0x0f) << 12) | ((second & 0x3f) << 6) | (third & 0x3f));
          continue;
        }
      } else if (first >= 0xf0 && first <= 0xf4 && index + 2 < bytes.length) {
        const second = bytes[index++];
        const third = bytes[index++];
        const fourth = bytes[index++];
        const validSecond =
          continuation(second) &&
          (first !== 0xf0 || second >= 0x90) &&
          (first !== 0xf4 || second <= 0x8f);
        if (validSecond && continuation(third) && continuation(fourth)) {
          append(
            ((first & 0x07) << 18) |
              ((second & 0x3f) << 12) |
              ((third & 0x3f) << 6) |
              (fourth & 0x3f),
          );
          continue;
        }
      }
      throw error("Invalid UTF-8", "InvalidData");
    }
    if (codePoints.length) output += String.fromCodePoint(...codePoints);
    return output;
  };

  const encodeUtf8 = (value) => {
    const output = [];
    for (const character of value) {
      const point = character.codePointAt(0);
      if (point <= 0x7f) output.push(point);
      else if (point <= 0x7ff)
        output.push(0xc0 | (point >> 6), 0x80 | (point & 0x3f));
      else if (point <= 0xffff)
        output.push(
          0xe0 | (point >> 12),
          0x80 | ((point >> 6) & 0x3f),
          0x80 | (point & 0x3f),
        );
      else
        output.push(
          0xf0 | (point >> 18),
          0x80 | ((point >> 12) & 0x3f),
          0x80 | ((point >> 6) & 0x3f),
          0x80 | (point & 0x3f),
        );
    }
    return new Uint8Array(output);
  };

  const readFile = (url, encoding) =>
    new Promise((resolve, reject) => {
      if (typeof url !== "string" || (encoding !== undefined && encoding !== "utf8")) {
        return reject(error("Invalid mount URI or encoding", "InvalidRequest"));
      }
      readFileNative(url, 4 * 1024 * 1024, resolve, reject);
    }).then((bytes) => {
      const value = new Uint8Array(bytes);
      return encoding === "utf8" ? decodeUtf8(value) : value;
    });
  const mutateMount = (kind, first, second = null) =>
    new Promise((resolve, reject) => {
      if (typeof first !== "string") {
        return reject(error("Invalid mount URI", "InvalidRequest"));
      }
      mountOperationNative(kind, first, second, resolve, reject);
    });
  const writeFile = (url, data) => {
    let bytes;
    if (typeof data === "string") bytes = encodeUtf8(data);
    else if (data instanceof Uint8Array) bytes = data;
    else return Promise.reject(error("Expected string or Uint8Array", "InvalidRequest"));
    return mutateMount(11, url, Array.from(bytes));
  };
  const mkdir = (url) => mutateMount(12, url);
  const removeFile = (url) => mutateMount(13, url);
  const removeDirectory = (url) => mutateMount(14, url);
  const rename = (from, to) => {
    if (typeof to !== "string") {
      return Promise.reject(error("Invalid mount URI", "InvalidRequest"));
    }
    return mutateMount(15, from, to);
  };
  Object.defineProperty(globalThis, "Zintl", {
    value: Object.freeze({
      invoke,
      sleep,
      readFile,
      writeFile,
      mkdir,
      removeFile,
      removeDirectory,
      rename,
    }),
    configurable: false,
  });

  const formatConsoleValue = (value) => {
    if (typeof value === "string") return value;
    try {
      const encoded = JSON.stringify(value);
      if (encoded !== undefined) return encoded;
    } catch (_) {}
    try {
      return String(value);
    } catch (_) {
      return "<unprintable>";
    }
  };
  const emitConsole = (...values) =>
    consoleNative(values.map(formatConsoleValue).join(" "));
  Object.defineProperty(globalThis, "console", {
    value: Object.freeze({
      debug: emitConsole,
      log: emitConsole,
      info: emitConsole,
      warn: emitConsole,
      error: emitConsole,
    }),
    configurable: false,
  });

  const encode = (value) =>
    JSON.stringify(
      value instanceof Uint8Array
        ? { type: "bytes", value: Array.from(value) }
        : { type: "value", value: value === undefined ? null : value },
    );
  const evaluate = (id, source) => {
    let result;
    try {
      result = (0, eval)(source);
    } catch (exception) {
      evaluationDone(id, false, `${exception.name}: ${exception.message}`);
      return;
    }
    Promise.resolve(result).then(
      (value) => evaluationDone(id, true, encode(value)),
      (exception) =>
        evaluationDone(
          id,
          false,
          `${exception?.name ?? "Error"}: ${exception?.message ?? exception}`,
        ),
    );
  };
  return Object.freeze({ evaluate });
})()
