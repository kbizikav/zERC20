declare module 'qrcode' {
  type ErrorCorrectionLevel = 'L' | 'M' | 'Q' | 'H';

  interface QRCodeColorOptions {
    dark?: string;
    light?: string;
  }

  interface QRCodeToDataURLOptions {
    errorCorrectionLevel?: ErrorCorrectionLevel;
    type?: 'image/png' | 'image/jpeg' | 'image/webp';
    margin?: number;
    scale?: number;
    width?: number;
    color?: QRCodeColorOptions;
  }

  export function toDataURL(text: string, options?: QRCodeToDataURLOptions): Promise<string>;
}
