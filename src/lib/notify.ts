import { platform } from './platform';

/** Show a system notification. The OS asks for permission the first time, if it needs to. */
export async function notify(title: string, body: string) {
  try {
    await platform.notify(title, body.length > 240 ? body.slice(0, 239) + '…' : body);
  } catch (e) {
    console.warn('notification failed', e);
  }
}
