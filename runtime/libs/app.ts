import { primordials } from "ext:core/mod.js";
import {
  op_zintl_app_next_event,
  op_zintl_app_set_commands,
  op_zintl_window_create,
} from "ext:core/ops";
import type { ZintlWindow, ZintlWindowCreateOptions } from "./window.ts";
import { createZintlWindow } from "./window.ts";

const { ObjectDefineProperty } = primordials;

interface NativeWindowEvent {
  type: "onload" | "willclose" | "click";
  windowId?: number;
  commandId?: string;
}

export interface ZintlApp {
  commands: ZintlAppCommands;
  createWindow(options?: ZintlWindowCreateOptions): Promise<ZintlWindow>;
}

export type ZintlCommandModifier = "cmd" | "ctrl" | "alt" | "shift";
export type ZintlCommandRole = "about" | "quit";

export interface ZintlCommandItem {
  id?: string;
  title: string;
  role?: ZintlCommandRole;
  key?: string;
  modifiers?: ZintlCommandModifier[];
  enabled?: boolean;
}

export interface ZintlAppMenu {
  items: ZintlCommandItem[];
}

export interface ZintlCommandMenu {
  title: string;
  items: ZintlCommandItem[];
}

export interface ZintlAppCommands {
  appMenu?: ZintlAppMenu;
  menus?: ZintlCommandMenu[];
}

let commands: ZintlAppCommands = {};
let isListeningForNativeEvents = false;

const app: ZintlApp = {
  get commands(): ZintlAppCommands {
    return commands;
  },

  set commands(value: ZintlAppCommands) {
    op_zintl_app_set_commands(value);
    commands = value;
  },

  async createWindow(
    options?: ZintlWindowCreateOptions,
  ): Promise<ZintlWindow> {
    ensureNativeEventListening();
    return createZintlWindow(
      await op_zintl_window_create(options ?? null),
    );
  },
};

ObjectDefineProperty(globalThis, "app", {
  value: app,
  configurable: true,
  enumerable: false,
  writable: true,
});

async function listenForNativeEvents(): Promise<void> {
  while (true) {
    const nativeEvent: NativeWindowEvent = await op_zintl_app_next_event();
    const event = new CustomEvent(nativeEvent.type, { detail: nativeEvent });
    if (nativeEvent.windowId !== undefined) {
      ObjectDefineProperty(event, "windowId", {
        value: nativeEvent.windowId,
        enumerable: true,
      });
    }
    if (nativeEvent.commandId !== undefined) {
      ObjectDefineProperty(event, "commandId", {
        value: nativeEvent.commandId,
        enumerable: true,
      });
    }
    globalThis.dispatchEvent(event);
  }
}

function ensureNativeEventListening(): void {
  if (isListeningForNativeEvents) {
    return;
  }
  isListeningForNativeEvents = true;
  listenForNativeEvents();
}

export { app };
