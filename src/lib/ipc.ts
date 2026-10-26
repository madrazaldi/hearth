// The bridge between the UI and the backend, with the shape the rest of the
// app expects: `invoke` runs a named command, `listen` subscribes to pushes.
//
// Inside the app the transport is Electrobun's RPC to the Bun process. In a
// plain browser (`bun run dev`) there is no backend, so src/lib/mock.ts
// registers a scripted one through `installMock`.

import type { PushEvents } from '../shared/rpc';

export type Unlisten = () => void;
type Handler = (payload: unknown) => void;
type Mock = (cmd: string, args: Record<string, unknown>) => unknown | Promise<unknown>;

const listeners = new Map<string, Set<Handler>>();

function dispatch(event: string, payload: unknown) {
  for (const h of [...(listeners.get(event) ?? [])]) {
    try {
      h(payload);
    } catch (e) {
      console.error(`listener for ${event} failed`, e);
    }
  }
}

let mock: Mock | null = null;

/** Replace the backend with a scripted one (browser preview and tests). */
export function installMock(handler: Mock) {
  mock = handler;
}

/** Push an event the way the backend would. */
export function emitMock(event: string, payload: unknown) {
  dispatch(event, payload);
}

/** Whether the UI is running inside the desktop app. */
export function inApp(): boolean {
  return typeof window !== 'undefined' && window.__electrobunWebviewId !== undefined;
}

type Rpc = { request: (cmd: string, params?: unknown) => Promise<unknown> };
let bridge: Promise<Rpc> | null = null;

function connectBridge(): Promise<Rpc> {
  return (bridge ??= (async () => {
    const { Electroview } = await import('electrobun/view');
    const rpc = Electroview.defineRPC({
      // Commands such as history requests and uploads outlive the default one-second limit.
      maxRequestTime: Infinity,
      handlers: {
        requests: {},
        messages: {
          push: ({ event, payload }: { event: string; payload: unknown }) => dispatch(event, payload),
        },
      },
      // The shared schema is left open: commands are plain names, not a closed set.
    } as never);
    new Electroview({ rpc });
    return rpc as unknown as Rpc;
  })());
}

/** Run a backend command. Failures reject with the message string, as the UI has always seen them. */
export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    if (mock) return (await mock(cmd, args)) as T;
    if (!inApp()) throw new Error('no backend is available');
    const rpc = await connectBridge();
    return (await rpc.request(cmd, args)) as T;
  } catch (e) {
    throw e instanceof Error ? e.message : String(e);
  }
}

/** Subscribe to a push event. In the app this also makes sure the bridge is up. */
export async function listen<K extends keyof PushEvents>(event: K, cb: (payload: PushEvents[K]) => void): Promise<Unlisten> {
  let set = listeners.get(event);
  if (!set) listeners.set(event, (set = new Set()));
  const handler = cb as Handler;
  set.add(handler);
  if (!mock && inApp()) await connectBridge();
  return () => void set.delete(handler);
}
