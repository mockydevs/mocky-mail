/** The size that fits `width` × `height` inside `max` × `max`, keeping its shape (never larger). */
export function fitWithin(width: number, height: number, max = 256): { width: number; height: number } {
  if (width <= 0 || height <= 0) return { width: max, height: max };
  const scale = Math.min(1, max / Math.max(width, height));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

/** The largest picture kept with a contact, in bytes of data URL. */
export const MAX_PHOTO_CHARS = 200_000;

function loadImage(file: File): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve(img);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("That file isn't a picture this device can read."));
    };
    img.src = url;
  });
}

/**
 * A picked image as a small contact photo: scaled down to fit 256 × 256 on
 * the device and encoded as a JPEG `data:` URL (on white, for transparent
 * pictures).
 */
export async function photoFromFile(file: File, max = 256): Promise<string> {
  if (!file.type.startsWith('image/')) throw new Error('Choose a picture (JPEG, PNG, GIF or WebP).');
  const img = await loadImage(file);
  const size = fitWithin(img.naturalWidth, img.naturalHeight, max);
  const canvas = document.createElement('canvas');
  canvas.width = size.width;
  canvas.height = size.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error("This device can't resize pictures.");
  ctx.fillStyle = '#ffffff';
  ctx.fillRect(0, 0, size.width, size.height);
  ctx.drawImage(img, 0, 0, size.width, size.height);
  let quality = 0.85;
  let url = canvas.toDataURL('image/jpeg', quality);
  while (url.length > MAX_PHOTO_CHARS && quality > 0.4) {
    quality -= 0.15;
    url = canvas.toDataURL('image/jpeg', quality);
  }
  return url;
}
