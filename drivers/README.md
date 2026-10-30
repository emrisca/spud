# Driver packs

Spud does not ship, bundle or install virtual-cable drivers. This folder is
where **you** put the installers you have downloaded, so a machine setup has
them to hand.

Nothing in this folder is committed: `*.zip`, `*.exe`, `*.msi` and `*.dll` are
git-ignored, and the folder itself is left out of the repository. That is
deliberate — see below.

## Why these are not in the repository

- **Their licence is per PC and redistribution needs a distribution licence.**
  [VB-Audio licensing](https://vb-audio.com/Services/licensing.htm) treats A+B
  and C+D as donationware *for downloading*, and requires purchased licences to
  resell or distribute. Shipping the installers in a build, or committing them
  into a fork, would be redistributing someone else's signed driver.
- **A+B and C+D have no public download.** VB-Audio issues a personal download
  link per purchase, stored in the webshop account (`My Download`). There is
  nothing to fetch without an account, so no automatic `curl` can populate this
  folder.
- The drivers are signed kernel drivers. Putting binaries in git makes a clone
  heavier and invites "just run this driver from the internet" habits.

The plain **VB-CABLE** is free and directly downloadable
(`VBCABLE_Driver_Pack45.zip` from <https://vb-audio.com/Cable/>); A+B and C+D are
the ones that need the webshop link.

## What goes here

| File (as shipped) | Cables it installs | Spud channels |
| --- | --- | --- |
| `VBCABLE_Driver_Pack45.zip` | `CABLE` | Virtual 1 / B1 |
| `VBCABLE_A_B_Driver_Pack45.zip` | `CABLE-A`, `CABLE-B` | Virtual 2, Virtual 3 / B2, B3 |
| `VBCable_CD_PackSetup_v2152.zip` | `CABLE-C`, `CABLE-D` | none yet — the console has three virtual channels |

## Installing

1. Extract each package and run its `Setup` program **as administrator**.
2. Reboot. The drivers are kernel-mode; the endpoints do not appear until then.
3. Open Spud → **Settings → Rescan devices**. Each cable's capture side is
   wired to a virtual strip and its render side to the matching B bus, paired by
   cable name rather than enumeration order.
4. Check the strip's device label: `CABLE Output`, `CABLE-A Output`,
   `CABLE-B Output`.

Uninstalling is the same in reverse, and needs a reboot too. Removing
Voicemeeter does **not** remove these cables; they are separate drivers.

## Cables Spud cannot use yet

An A+B **and** C+D install gives five cables, but the console has three virtual
inputs and three virtual B buses (`CABLE` → Virtual 1, `CABLE-A` → Virtual 2,
`CABLE-B` → Virtual 3). `CABLE-C` and `CABLE-D` still appear in every device
picker and can be assigned to a hardware strip and an A bus by hand — they just
do not get the virtual panel, the app list, or automatic pairing.
