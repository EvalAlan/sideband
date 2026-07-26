# Sideband — Privacy Policy

_Last updated: 2026-07-24_

Sideband is a peer-to-peer messenger. It has no accounts, no servers operated by
us, and no analytics. This document describes exactly what the app does with
your data, including the things it *cannot* protect.

## The short version

- **We collect nothing.** There is no Sideband server, no account, no sign-up,
  no phone number or email, and no telemetry, analytics, ads, or crash
  reporting.
- **Your data stays on your device.** Identity keys, contacts, and message
  history are stored locally in the app's private storage.
- **Messages are end-to-end encrypted** and travel directly between devices.

## What is stored on your device

- **Your identity keys** (Ed25519 signing key, X25519 encryption key).
- **Your contacts** — the name you give them and their public keys and
  addresses.
- **Your message history**, including any files you send or receive.
- **Session state** for forward secrecy (Double Ratchet), and, if you use
  multiple devices, your signed device list.

You can encrypt all of this at rest with a passphrase (**Settings → App lock**);
the passphrase is never stored, only a key derived from it is used in memory.
**Panic — delete everything** (in Settings) irreversibly erases all of the above.

## What leaves your device, and who can see it

Messages are end-to-end encrypted and sent directly to your contacts over
whichever route is available:

- **Tor onion services** (over the internet). Tor relays carry encrypted
  traffic and do not see message contents. Use of the Tor network is subject to
  that network's own operation; we do not run it.
- **Local Wi-Fi** and **Bluetooth**, when the contact is nearby. These are
  direct device-to-device connections; nothing is relayed through us.

**Your contacts** necessarily see the messages you send them. If you enable them,
they may also receive **read receipts**, **presence** ("online"/"away"), and a
**status message**. These are optional and controlled in Settings.

We never receive any of this, because there is no server to receive it.

## Permissions and why they are needed

| Permission | Why |
|---|---|
| Internet | Connecting to the Tor network to reach contacts |
| Bluetooth (connect / scan) | Delivering messages to a nearby contact without internet. Scanning is declared `neverForLocation` and is **not** used to determine your location. |
| Camera | Scanning a contact's QR code. Images are processed on-device and not stored or transmitted. |
| Notifications | Telling you about new messages |
| Foreground service | Keeping the connection alive so messages can arrive while the app is in the background |

## Third-party components

Sideband is open source and self-contained, with these exceptions worth naming:

- **The Tor network** — used to route internet traffic. Operated by volunteers,
  not by us.
- **Barcode scanning** — the Android build currently uses Google's ML Kit
  barcode scanner for QR scanning. It runs **on-device**; the app does not send
  images anywhere. Depending on the build, this component may come from Google
  Play Services.

There are no advertising, analytics, or tracking SDKs.

## What Sideband does *not* protect against

Being honest about limits matters more than marketing:

- **Someone with access to your unlocked device** can read your messages. Use
  App lock, and a device lock screen.
- **Your contacts** can keep, screenshot, or forward anything you send them.
  Disappearing messages are a convenience, not an enforcement mechanism.
- **Bluetooth and Wi-Fi are local radios.** Using them reveals that a device is
  present and transmitting to anyone in range, even though the contents are
  encrypted.
- **Sideband has not been independently security-audited.** It is early
  software. Do not rely on it where your safety depends on it.

## Children

Sideband is not directed at children and does not knowingly collect information
from anyone — because it does not collect information from anyone at all.

## Changes

Changes to this policy will be published in this file in the project
repository, with the date above updated.

## Contact

Questions or reports: open an issue at
<https://github.com/EvalAlan/sideband>.
