import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';

let permitted: boolean | null = null;

export async function notify(title: string, body: string) {
  try {
    if (permitted === null) {
      permitted = await isPermissionGranted();
      if (!permitted) permitted = (await requestPermission()) === 'granted';
    }
    if (permitted) sendNotification({ title, body: body.length > 240 ? body.slice(0, 239) + '…' : body });
  } catch (e) {
    console.warn('notification failed', e);
  }
}
