import { primordials } from "ext:core/mod.js";
import { op_zintl_app_event_bus_poll } from "ext:core/ops";

const {
  ObjectDefineProperty,
  SafeSet,
  SafeSetIterator,
  SetPrototypeAdd,
  SetPrototypeDelete,
  SetPrototypeGetSize,
} = primordials;

export interface AppWindowCommandEvent {
  type: "window.command";
  windowId: number;
  commandId: string;
}

export interface AppWindowCreatedEvent {
  type: "window.created";
  windowId: number;
}

export interface AppWindowWillCloseEvent {
  type: "window.willClose";
  windowId: number;
}

export type AppEvent =
  | AppWindowCommandEvent
  | AppWindowCreatedEvent
  | AppWindowWillCloseEvent;
export type AppEventType = AppEvent["type"];
export type AppEventListener<T extends AppEventType = AppEventType> = (
  event: Extract<AppEvent, { type: T }>,
) => void;

export interface AppEventBus {
  subscribe<T extends AppEventType>(
    type: T,
    listener: AppEventListener<T>,
  ): () => void;
  poll(): AppEvent | undefined;
}

export interface ZintlApp {
  eventBus: AppEventBus;
}

type ZintlGlobalThis = typeof globalThis & {
  app?: ZintlApp;
};

interface AppEventSubscription {
  type: AppEventType;
  listener: (event: AppEvent) => void;
}

const appEventSubscriptions = new SafeSet<AppEventSubscription>();
let appEventPollTimer: number | undefined;

export const eventBus: AppEventBus = {
  subscribe<T extends AppEventType>(
    type: T,
    listener: AppEventListener<T>,
  ): () => void {
    const subscription: AppEventSubscription = {
      type,
      listener: (event) => listener(event as Extract<AppEvent, { type: T }>),
    };
    SetPrototypeAdd(appEventSubscriptions, subscription);
    ensureAppEventPolling();

    return () => {
      SetPrototypeDelete(appEventSubscriptions, subscription);
      if (
        SetPrototypeGetSize(appEventSubscriptions) === 0 &&
        appEventPollTimer !== undefined
      ) {
        globalThis.clearInterval(appEventPollTimer);
        appEventPollTimer = undefined;
      }
    };
  },

  poll(): AppEvent | undefined {
    return op_zintl_app_event_bus_poll() ?? undefined;
  },
};

function ensureAppEventPolling(): void {
  if (appEventPollTimer !== undefined) {
    return;
  }

  appEventPollTimer = globalThis.setInterval(() => {
    let event: AppEvent | undefined;
    while ((event = eventBus.poll()) != null) {
      dispatchAppEvent(event);
    }
  }, 16);
}

function dispatchAppEvent(event: AppEvent): void {
  for (const subscription of new SafeSetIterator(appEventSubscriptions)) {
    if (subscription.type === event.type) {
      subscription.listener(event);
    }
  }
}

const zintlGlobalThis = globalThis as ZintlGlobalThis;
const app = zintlGlobalThis.app ?? { eventBus };
app.eventBus = eventBus;

ObjectDefineProperty(globalThis, "app", {
  value: app,
  configurable: true,
  enumerable: false,
  writable: true,
});

export { app };
