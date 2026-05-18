import { primordials } from "ext:core/mod.js";
import { op_zintl_window_create } from "ext:core/ops";

const { ObjectDefineProperty } = primordials;
const Zintl = globalThis.Zintl ?? {};

export interface ZintlWindowAPI {
    create(): void;
}

const windowApi: ZintlWindowAPI = {
  create() {
    op_zintl_window_create();
  },
};

Zintl.window = windowApi;

ObjectDefineProperty(globalThis, "Zintl", {
  value: Zintl,
  configurable: true,
  enumerable: false,
  writable: true,
});
