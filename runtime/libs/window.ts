import { primordials } from "ext:core/mod.js";
import {
  op_zintl_window_set_bounds,
  op_zintl_window_set_position,
  op_zintl_window_set_size,
} from "ext:core/ops";

const {
  ArrayPrototypePush,
  MapPrototypeDelete,
  MapPrototypeGet,
  MapPrototypeSet,
  queueMicrotask,
  SafeArrayIterator,
  SafeMap,
} = primordials;

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

export interface ZintlWindowCreateOptions {
  bounds?: ZintlWindowBounds;
  size?: ZintlWindowSize;
  position?: ZintlWindowPosition;
}

export interface ZintlWindowClickEvent extends
  CustomEvent<{
    type: "click";
    windowId: number;
  }> {
  readonly windowId: number;
}

export type ZintlWindowClickListener = (
  event: ZintlWindowClickEvent,
) => void;

export interface ZintlWindowLifecycleEvent extends
  CustomEvent<{
    type: "onload" | "willclose";
    windowId: number;
  }> {
  readonly windowId: number;
}

export type ZintlWindowLifecycleListener = (
  event: ZintlWindowLifecycleEvent,
) => void;

export interface ZintlWindow {
  readonly id: number;
  addEventListener(
    type: "onload" | "willclose",
    listener: ZintlWindowLifecycleListener | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  addEventListener(
    type: "click",
    listener: ZintlWindowClickListener | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  removeEventListener(
    type: "onload" | "willclose",
    listener: ZintlWindowLifecycleListener | null,
    options?: boolean | EventListenerOptions,
  ): void;
  removeEventListener(
    type: "click",
    listener: ZintlWindowClickListener | null,
    options?: boolean | EventListenerOptions,
  ): void;
  setBounds(bounds: ZintlWindowBounds): Promise<void>;
  setSize(size: ZintlWindowSize): Promise<void>;
  setPosition(position: ZintlWindowPosition): Promise<void>;
}

const windows = new SafeMap<number, NativeZintlWindow>();
const pendingEvents = new SafeMap<number, Event[]>();

class NativeZintlWindow extends EventTarget implements ZintlWindow {
  #id: number;

  constructor(id: number) {
    super();
    this.#id = id;
  }

  get id(): number {
    return this.#id;
  }

  override addEventListener(
    type: "onload" | "willclose",
    listener: ZintlWindowLifecycleListener | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  override addEventListener(
    type: "click",
    listener: ZintlWindowClickListener | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  override addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  override addEventListener(
    type: string,
    listener:
      | EventListenerOrEventListenerObject
      | ZintlWindowClickListener
      | ZintlWindowLifecycleListener
      | null,
    options?: boolean | AddEventListenerOptions,
  ): void {
    super.addEventListener(type, listener as EventListener, options);
  }

  override removeEventListener(
    type: "onload" | "willclose",
    listener: ZintlWindowLifecycleListener | null,
    options?: boolean | EventListenerOptions,
  ): void;
  override removeEventListener(
    type: "click",
    listener: ZintlWindowClickListener | null,
    options?: boolean | EventListenerOptions,
  ): void;
  override removeEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | EventListenerOptions,
  ): void;
  override removeEventListener(
    type: string,
    listener:
      | EventListenerOrEventListenerObject
      | ZintlWindowClickListener
      | ZintlWindowLifecycleListener
      | null,
    options?: boolean | EventListenerOptions,
  ): void {
    super.removeEventListener(type, listener as EventListener, options);
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
}

export function createZintlWindow(id: number): ZintlWindow {
  const window = new NativeZintlWindow(id);
  MapPrototypeSet(windows, id, window);
  const pending = MapPrototypeGet(pendingEvents, id);
  if (pending !== undefined) {
    MapPrototypeDelete(pendingEvents, id);
    queueMicrotask(() => {
      queueMicrotask(() => {
        for (const event of new SafeArrayIterator(pending)) {
          window.dispatchEvent(event);
        }
      });
    });
  }
  return window;
}

export function dispatchZintlWindowEvent(
  windowId: number,
  event: Event,
): void {
  const window = MapPrototypeGet(windows, windowId);
  if (window !== undefined) {
    window.dispatchEvent(event);
    return;
  }

  let pending = MapPrototypeGet(pendingEvents, windowId);
  if (pending === undefined) {
    pending = [];
    MapPrototypeSet(pendingEvents, windowId, pending);
  }
  ArrayPrototypePush(pending, event);
}

export function forgetZintlWindow(windowId: number): void {
  MapPrototypeDelete(windows, windowId);
  MapPrototypeDelete(pendingEvents, windowId);
}
