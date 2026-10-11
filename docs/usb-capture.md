# Recording what the vendor software sends

**English** · [Русский](usb-capture.ru.md)

If nobody has documented your device's lighting protocol, it can be learned from a recording of what the vendor software (G HUB, Synapse, iCUE, SteelSeries GG…) sends to the device while you change the lighting. This guide shows how to make such a recording on Windows with Wireshark and USBPcap. It takes about 15 minutes.

## Before you start: privacy

A USB recording contains **everything** the recorded device sends, including the keys you press on a keyboard.

- Record **only the device you are asking about**, not the whole USB bus (step 3).
- While recording, **use only the mouse** to click through the vendor program. Don't type anything, least of all passwords.
- If you have to type (for example, to name a profile), stop the recording first.

## 1. Install Wireshark with USBPcap

1. Download Wireshark from [wireshark.org](https://www.wireshark.org/download.html) (Windows x64 Installer).
2. In the installer, tick **Install USBPcap** on the USBPcap page.
3. **Restart Windows.** USBPcap is a driver and only works after a restart.

## 2. Prepare

- **Quit RGB Tray** (tray icon → Exit). Otherwise the recording mixes its commands with the vendor's.
- Quit other lighting programs too (SignalRGB, OpenRGB) and keep only the vendor software for your device.
- Set up the vendor program so that you can change the lighting in a few clicks.
- Have something to take notes in: you'll write down what you changed and when.

## 3. Find your device

1. Start Wireshark **as administrator**.
2. The start screen lists interfaces `USBPcap1`, `USBPcap2`, …, one per USB root hub. Click the **gear icon** next to one of them.
3. The window shows the devices on that hub. Find yours by name. If you're unsure, unplug the device, see which line disappears, then plug it back in. A wireless device is recorded through its receiver (dongle).
4. Tick **only your device**, untick **Capture from all devices connected** and **Capture from newly connected devices**. Click **Start**.

If the device isn't on the first hub, stop, and try the gear of `USBPcap2` and so on.

## 4. Record

Do **one change at a time** and wait **about 5 seconds** between changes, so that each change is a separate burst in the recording. Write down what you did, in order. A good sequence:

1. Wait 10 seconds without touching anything (idle traffic).
2. Static color **red** (FF0000).
3. Static color **green** (00FF00).
4. Static color **blue** (0000FF).
5. Brightness **100%**, then **50%**, then **0%**, then back to 100%.
6. Effect **breathing**, then **spectrum / color cycle**, then **off**.
7. If the device has zones (logo, wheel, side): a different color on **one zone only**.
8. If it has per-key lighting: **one key** in a different color.
9. If an effect has a speed setting: slowest, then fastest.

Pure colors (red, green, blue) are easy to spot in the bytes, which is why they are worth the effort.

## 5. Save

1. Stop the recording (red square).
2. **File → Save As**, format **pcapng**. Name it after the device, for example `naga-v2-pro.pcapng`.

## 6. Send it

Attach the file to your [device request](../../../issues/new?template=device-request.yml) (zip it first if GitHub refuses the extension), together with:

- your notes, i.e. what you changed, in order;
- the vendor program and its version;
- the device report from RGB Tray (tray icon → Request device support…).

## Optional: look at it yourself

The lighting commands are usually transfers *to* the device:

- feature and output reports over the control endpoint: display filter `usb.setup.bRequest == 9` (HID `SET_REPORT`);
- interrupt transfers to the device: `usb.transfer_type == 0x01 && usb.endpoint_address.direction == 0`.

Click a packet and look at the bytes at the bottom. A color usually appears as three bytes in a row, for example `ff 00 00` for red. Compare the packets of two changes that differ in one thing: the bytes that differ are the parameter, and the ones that stay the same are the command.
