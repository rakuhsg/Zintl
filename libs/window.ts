import { primordials } from "ext:core/mod.js";
import {
  op_zintl_window_create,
  op_zintl_window_set_bounds,
  op_zintl_window_set_commands,
  op_zintl_window_set_position,
  op_zintl_window_set_size,
  op_zintl_window_take_command_event,
  op_zintl_window_take_lifecycle_event,
} from "ext:core/ops";

const {
  ObjectDefineProperty,
  SafeSet,
  SafeSetIterator,
  SetPrototypeAdd,
  SetPrototypeDelete,
  SetPrototypeGetSize,
} = primordials;
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
  setBounds(bounds: ZintlWindowBounds): void;
  setSize(size: ZintlWindowSize): void;
  setPosition(position: ZintlWindowPosition): void;
  setCommands(commands: ZintlWindowCommandSet): void;
  onCommand(listener: ZintlWindowCommandListener): () => void;
  onCreated(listener: ZintlWindowLifecycleListener): () => void;
  onWillClose(listener: ZintlWindowLifecycleListener): () => void;
}

export interface ZintlWindowAPI {
  create(options?: ZintlWindowCreateOptions): ZintlWindow;
}

const commandListeners = new SafeSet<ZintlWindowCommandListener>();
let commandPollTimer: number | undefined;
const createdListeners = new SafeSet<ZintlWindowLifecycleListener>();
const willCloseListeners = new SafeSet<ZintlWindowLifecycleListener>();
let lifecyclePollTimer: number | undefined;

class NativeZintlWindow implements ZintlWindow {
  #id: number;

  constructor(id: number) {
    this.#id = id;
  }

  get id(): number {
    return this.#id;
  }

  setBounds(bounds: ZintlWindowBounds): void {
    op_zintl_window_set_bounds(this.#id, bounds);
  }

  setSize(size: ZintlWindowSize): void {
    op_zintl_window_set_size(this.#id, size);
  }

  setPosition(position: ZintlWindowPosition): void {
    op_zintl_window_set_position(this.#id, position);
  }

  setCommands(commands: ZintlWindowCommandSet): void {
    op_zintl_window_set_commands(this.#id, commands);
  }

  onCommand(listener: ZintlWindowCommandListener): () => void {
    SetPrototypeAdd(commandListeners, listener);
    ensureCommandPolling();
    return () => {
      SetPrototypeDelete(commandListeners, listener);
      if (SetPrototypeGetSize(commandListeners) === 0 && commandPollTimer !== undefined) {
        globalThis.clearInterval(commandPollTimer);
        commandPollTimer = undefined;
      }
    };
  }

  onCreated(listener: ZintlWindowLifecycleListener): () => void {
    const windowId = this.#id;
    return addLifecycleListener(createdListeners, (event) => {
      if (event.windowId === windowId) {
        listener(event);
      }
    });
  }

  onWillClose(listener: ZintlWindowLifecycleListener): () => void {
    const windowId = this.#id;
    return addLifecycleListener(willCloseListeners, (event) => {
      if (event.windowId === windowId) {
        listener(event);
      }
    });
  }
}

const windowApi: ZintlWindowAPI = {
  create(options?: ZintlWindowCreateOptions): ZintlWindow {
    return new NativeZintlWindow(op_zintl_window_create(options ?? null));
  },
};

function ensureCommandPolling(): void {
  if (commandPollTimer !== undefined) {
    return;
  }

  commandPollTimer = globalThis.setInterval(() => {
    let event;
    while ((event = op_zintl_window_take_command_event()) != null) {
      for (const listener of new SafeSetIterator(commandListeners)) {
        listener(event);
      }
    }
  }, 16);
}

function addLifecycleListener(
  listeners: SafeSet<ZintlWindowLifecycleListener>,
  listener: ZintlWindowLifecycleListener,
): () => void {
  SetPrototypeAdd(listeners, listener);
  ensureLifecyclePolling();
  return () => {
    SetPrototypeDelete(listeners, listener);
    if (
      SetPrototypeGetSize(createdListeners) === 0 &&
      SetPrototypeGetSize(willCloseListeners) === 0 &&
      lifecyclePollTimer !== undefined
    ) {
      globalThis.clearInterval(lifecyclePollTimer);
      lifecyclePollTimer = undefined;
    }
  };
}

function ensureLifecyclePolling(): void {
  if (lifecyclePollTimer !== undefined) {
    return;
  }

  lifecyclePollTimer = globalThis.setInterval(() => {
    let event;
    while ((event = op_zintl_window_take_lifecycle_event()) != null) {
      const listeners = event.kind === "created" ? createdListeners : willCloseListeners;
      for (const listener of new SafeSetIterator(listeners)) {
        listener({ windowId: event.windowId });
      }
    }
  }, 16);
}

Zintl.window = windowApi;

ObjectDefineProperty(globalThis, "Zintl", {
  value: Zintl,
  configurable: true,
  enumerable: false,
  writable: true,
});
