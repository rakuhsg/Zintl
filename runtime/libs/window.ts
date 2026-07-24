import {
  op_zintl_window_set_bounds,
  op_zintl_window_set_position,
  op_zintl_window_set_size,
} from "ext:core/ops";

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

export interface ZintlWindow {
  readonly id: number;
  setBounds(bounds: ZintlWindowBounds): Promise<void>;
  setSize(size: ZintlWindowSize): Promise<void>;
  setPosition(position: ZintlWindowPosition): Promise<void>;
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
}

export function createZintlWindow(id: number): ZintlWindow {
  return new NativeZintlWindow(id);
}
