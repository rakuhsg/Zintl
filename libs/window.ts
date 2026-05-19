import { primordials } from "ext:core/mod.js";
import {
  op_zintl_window_create,
  op_zintl_window_set_bounds,
  op_zintl_window_set_commands,
  op_zintl_window_set_position,
  op_zintl_window_set_size,
} from "ext:core/ops";
import { eventBus } from "ext:zintl/app.ts";

const { ObjectDefineProperty } = primordials;
const Zintl = globalThis.Zintl ?? {};

export interface ZintlWindowBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ZintlWindowSize {
  width: number;
  height: number;
}

export interface ZintlWindowPosition {
  x: number;
  y: number;
}

export type ZintlWindowCommandModifier = "cmd" | "ctrl" | "alt" | "shift";
export type ZintlWindowCommandRole = "about" | "quit";

export interface ZintlWindowCommandItem {
  id?: string;
  title: string;
  role?: ZintlWindowCommandRole;
  key?: string;
  modifiers?: ZintlWindowCommandModifier[];
  enabled?: boolean;
}

export interface ZintlWindowAppMenu {
  items: ZintlWindowCommandItem[];
}

export interface ZintlWindowCommandMenu {
  title: string;
  items: ZintlWindowCommandItem[];
}

export interface ZintlWindowCommandSet {
  appMenu?: ZintlWindowAppMenu;
  menus?: ZintlWindowCommandMenu[];
}

export interface ZintlWindowCreateOptions {
  bounds?: ZintlWindowBounds;
  size?: ZintlWindowSize;
  position?: ZintlWindowPosition;
  commands?: ZintlWindowCommandSet;
}

export interface ZintlWindowCommandEvent {
  windowId: number;
  commandId: string;
}

export interface ZintlWindowLifecycleEvent {
  windowId: number;
}

export type ZintlWindowCommandListener = (event: ZintlWindowCommandEvent) => void;
export type ZintlWindowLifecycleListener = (event: ZintlWindowLifecycleEvent) => void;

export interface ZintlWindow {
  readonly id: number;
  setBounds(bounds: ZintlWindowBounds): Promise<void>;
  setSize(size: ZintlWindowSize): Promise<void>;
  setPosition(position: ZintlWindowPosition): Promise<void>;
  setCommands(commands: ZintlWindowCommandSet): Promise<void>;
  onCommand(listener: ZintlWindowCommandListener): () => void;
  onCreated(listener: ZintlWindowLifecycleListener): () => void;
  onWillClose(listener: ZintlWindowLifecycleListener): () => void;
}

export interface ZintlWindowAPI {
  create(options?: ZintlWindowCreateOptions): Promise<ZintlWindow>;
}

class NativeZintlWindow implements ZintlWindow {
  #id: number;

  constructor(id: number) {
    this.#id = id;
  }

  get id(): number {
    return this.#id;
  }

  async setBounds(bounds: ZintlWindowBounds): Promise<void> {
    await op_zintl_window_set_bounds(this.#id, bounds);
  }

  async setSize(size: ZintlWindowSize): Promise<void> {
    await op_zintl_window_set_size(this.#id, size);
  }

  async setPosition(position: ZintlWindowPosition): Promise<void> {
    await op_zintl_window_set_position(this.#id, position);
  }

  async setCommands(commands: ZintlWindowCommandSet): Promise<void> {
    await op_zintl_window_set_commands(this.#id, commands);
  }

  onCommand(listener: ZintlWindowCommandListener): () => void {
    const windowId = this.#id;
    return eventBus.subscribe("window.command", (event) => {
      if (event.windowId === windowId) {
        listener({ windowId: event.windowId, commandId: event.commandId });
      }
    });
  }

  onCreated(listener: ZintlWindowLifecycleListener): () => void {
    const windowId = this.#id;
    return eventBus.subscribe("window.created", (event) => {
      if (event.windowId === windowId) {
        listener({ windowId: event.windowId });
      }
    });
  }

  onWillClose(listener: ZintlWindowLifecycleListener): () => void {
    const windowId = this.#id;
    return eventBus.subscribe("window.willClose", (event) => {
      if (event.windowId === windowId) {
        listener({ windowId: event.windowId });
      }
    });
  }
}

const windowApi: ZintlWindowAPI = {
  async create(options?: ZintlWindowCreateOptions): Promise<ZintlWindow> {
    return new NativeZintlWindow(await op_zintl_window_create(options ?? null));
  },
};

Zintl.window = windowApi;

ObjectDefineProperty(globalThis, "Zintl", {
  value: Zintl,
  configurable: true,
  enumerable: false,
  writable: true,
});
