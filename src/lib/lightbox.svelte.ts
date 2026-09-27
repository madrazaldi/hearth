// The one full-size image overlay, opened from any inline image. Kept outside
// the rows so a virtualized row unmounting doesn't close it.

export interface LightboxImage {
  src: string;
  url: string;
  width: number | null;
  height: number | null;
}

export const lightbox = $state<{ current: LightboxImage | null }>({ current: null });
