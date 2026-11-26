/** agent-browser's device presets (`native/actions.rs` `device_preset`). */
export interface DevicePreset {
  name: string;
  width: number;
  height: number;
  deviceScaleFactor: number;
  mobile: boolean;
  userAgent: string;
}

const IOS = (os: string, version: string) =>
  `Mozilla/5.0 (iPhone; CPU iPhone OS ${os} like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/${version} Mobile/15E148 Safari/604.1`;
const IPAD = 'Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/604.1';
const ANDROID = (android: string, model: string, chrome: string) =>
  `Mozilla/5.0 (Linux; Android ${android}; ${model}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${chrome} Mobile Safari/537.36`;

export const DEVICES: DevicePreset[] = [
  { name: 'iPhone 15', width: 393, height: 852, deviceScaleFactor: 3, mobile: true, userAgent: IOS('17_0', '17.0') },
  { name: 'iPhone 16', width: 393, height: 852, deviceScaleFactor: 3, mobile: true, userAgent: IOS('18_0', '18.0') },
  { name: 'iPhone 16 Pro', width: 402, height: 874, deviceScaleFactor: 3, mobile: true, userAgent: IOS('18_0', '18.0') },
  { name: 'iPhone 17', width: 402, height: 874, deviceScaleFactor: 3, mobile: true, userAgent: IOS('19_0', '19.0') },
  { name: 'iPad', width: 820, height: 1180, deviceScaleFactor: 2, mobile: true, userAgent: IPAD },
  { name: 'iPad Pro', width: 1024, height: 1366, deviceScaleFactor: 2, mobile: true, userAgent: IPAD },
  { name: 'Pixel 9', width: 412, height: 923, deviceScaleFactor: 2.625, mobile: true, userAgent: ANDROID('15', 'Pixel 9', '130.0.0.0') },
  { name: 'Galaxy S25', width: 360, height: 800, deviceScaleFactor: 3, mobile: true, userAgent: ANDROID('15', 'SM-S931B', '130.0.0.0') },
  { name: 'iPhone 12', width: 390, height: 844, deviceScaleFactor: 3, mobile: true, userAgent: IOS('14_0', '14.0') },
  { name: 'iPhone 14', width: 390, height: 844, deviceScaleFactor: 3, mobile: true, userAgent: IOS('16_0', '16.0') },
  { name: 'Pixel 5', width: 393, height: 851, deviceScaleFactor: 2.75, mobile: true, userAgent: ANDROID('11', 'Pixel 5', '90.0.4430.91') },
  { name: 'Pixel 7', width: 412, height: 915, deviceScaleFactor: 2.625, mobile: true, userAgent: ANDROID('13', 'Pixel 7', '116.0.0.0') },
  { name: 'Galaxy S21', width: 360, height: 800, deviceScaleFactor: 3, mobile: true, userAgent: ANDROID('11', 'SM-G991B', '90.0.4430.91') },
];

const squash = (name: string) => name.toLowerCase().replace(/\s+/g, '');

/** A preset by name, case and spaces ignored ("iphone15" is "iPhone 15"; "ipad air" is "iPad"). */
export function devicePreset(name: string): DevicePreset | undefined {
  const wanted = squash(name) === 'ipadair' ? 'ipad' : squash(name);
  return DEVICES.find((device) => squash(device.name) === wanted);
}
