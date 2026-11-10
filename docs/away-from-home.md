# Reach your Mac from away, and unlock it after a restart

Baton finds your devices on your own network, and goes through a relay when they cannot find each other. A relay is shared and slow, and a Mac that has just restarted is not running Baton at all. The second case is the one that matters most when you are away: the Mac restarted and is waiting for its disk password. This page sets up a private route into your home network, so that your phone reaches the Mac from anywhere as if it were at home, also at the screen where a restarted Mac waits.

Baton has no code for the route and does not look at which tool makes it. The steps below use Tailscale. Any tool that gives the phone a route to the Mac's address at home does the same job. [ADR 0017](adr/0017-away-from-home.md) has the reasons.

**This has not been tried on a Mac and a phone yet.** The steps follow what Tailscale and Apple document. The checks that settle what is still unknown are in [ADR 0017](adr/0017-away-from-home.md#how-it-will-be-measured).

## What it gives you, and what it does not

- **Unlock after a restart, from anywhere.** The **Unlock a Mac after restart** screen of the iPhone and iPad app talks to the Mac's SSH door, port 22, at an address of your home network. With the route, that address answers wherever the phone is.
- **Baton's own connection** (screen, control, files) tries every address that the Mac announces, and one of them is its home address, so it can use the route while the Mac is awake and signed in. That has not been tried either.
- **It does not wake a Mac that sleeps.** A Mac that is asleep answers nothing, with the route or without it.
- **It does not sign you in.** Whether the Mac signs the account in after its disk is unlocked, or waits at the login window, is not known yet. Baton does not run until someone is signed in.

## What you need

- A Tailscale account. Tailscale says that subnet routers are available on all of its plans.
- One device at home that stays on, runs Tailscale and is on the same network as the Mac. A small Linux computer such as a Raspberry Pi is the usual choice, and Tailscale says that almost any device that runs its client can do it. It cannot be the Mac you want to unlock, because that Mac runs nothing while it waits for its disk password.
- The Mac: macOS 26 or newer, FileVault turned on, Remote Login turned on, and a fixed address on your home network.
- The phone: the Tailscale app, and Baton.

## Set it up

### 1. Look at the range of your home network

Every device at home has an address that starts the same way, such as `192.168.1.x`. That start is the range. A café, a hotel or a friend's Wi-Fi that uses the same range as your home confuses the phone, because the address you saved then means a device there and not your Mac. The ranges `192.168.0.x` and `192.168.1.x` are the most common, so if your home uses one of them, that is the likeliest reason for a Mac that answers on some networks and not on others.

If your router lets you change the range (look for its LAN or DHCP settings), choose an unusual one such as `192.168.73.x`. Every device at home then takes a new address, which takes a few minutes. If you cannot change it, go on: the route works wherever the range is not shared. The rest of this page uses `192.168.73.0/24` as the range and `192.168.73.20` as the Mac's address. Replace them with yours.

### 2. Give the Mac a fixed address

In your router's settings, find the list of connected devices and reserve the address that the Mac has now. The setting is often called a DHCP reservation, an address reservation or a static lease. The Mac's address is in System Settings, under **Network**, your connection and **Details**. In Terminal, `ipconfig getifaddr en0` prints it for the Wi-Fi of most laptops, and prints nothing for a Mac that uses another connection.

Baton saves this address and not the Mac's name. A name such as `Jays-MBP.local` is announced on the local network and does not reach through the route, and it is not known whether a Mac that waits for its disk password announces one at all. Do not use an address that starts with `100.` either: that is a Tailscale address, and the Mac runs no Tailscale while it waits for its disk password.

### 3. Turn on Remote Login and check FileVault at the Mac

In System Settings, under **General** and **Sharing**, turn on **Remote Login**. Anyone who has the password of an account on the Mac can then sign in over SSH, so click the info button next to it and allow only the account that you unlock with. Under **Privacy & Security**, **FileVault** must be on: only then does the Mac ask for its disk password after a restart.

People report that a Mac had to be on Ethernet to answer before sign-in in the first version of macOS 26, and that Wi-Fi works from macOS 26.5. If the Mac does not answer at its disk password screen, try a cable.

### 4. Set up the device at home that carries the route

These are the commands for a Linux computer, as Tailscale's own pages give them. The page [Subnet routers](https://tailscale.com/kb/1019/subnets) has the current commands, and the steps for other systems.

```
curl -fsSL https://tailscale.com/install.sh | sh
echo 'net.ipv4.ip_forward = 1' | sudo tee -a /etc/sysctl.d/99-tailscale.conf
echo 'net.ipv6.conf.all.forwarding = 1' | sudo tee -a /etc/sysctl.d/99-tailscale.conf
sudo sysctl -p /etc/sysctl.d/99-tailscale.conf
sudo tailscale up --advertise-routes=192.168.73.0/24
```

Then open the **Machines** page of the Tailscale admin console, open the menu of this device, choose to edit its route settings and turn the route on. In the same menu, turn off the key expiry of this device, so that it does not need a new sign-in every few months, which would end the route while you are away.

The device must stay on, and stay on the same network as the Mac.

### 5. Set up the phone

Install Tailscale from the App Store, sign in with the same account and turn it on. Tailscale says that an iPhone takes an approved route by itself.

### 6. Save the Mac in Baton and check the way

In Baton on the phone, open **Settings**, **Unlock a Mac after restart** and **Add a Mac**. Type the Mac's fixed address, the account and its password, and tap **Check the Mac**. The phone shows the key that the Mac answered with. Compare it with the line that the command under it prints at the Mac, and save.

Then check the way from away: turn Wi-Fi off on the phone, so that it uses mobile data, make sure Tailscale is on, and tap **Check the Mac** on another **Add a Mac** screen with the same address. The check sends neither the account nor the password, so what you type in those two boxes does not matter for it. If the same key shows, the route works.

## If it does not work

| What the phone says | Likely reasons | What to try |
| --- | --- | --- |
| The Mac did not answer, only away from home | Tailscale is off on the phone. The device at home is off, or has lost its connection. The route is not turned on in the admin console. The network you are on uses the same range as your home. | In the Tailscale app, check that it is on and that the device at home shows as connected. Check the route in the admin console. Try another network. |
| The Mac did not answer, also at home | The Mac has a new address, because it was not reserved. Remote Login is off. The Mac is asleep. The Mac is on another network, such as a guest network. | Read the Mac's address again and compare it. Wake the Mac. Check Remote Login. |
| The Mac answered with another key | Another device has the address that you saved. Or the Mac waits for its disk password and answers with a key of its own, which is not known yet. | Do not trust it. Read the key of the Mac again while it runs. The restart test in [ADR 0017](adr/0017-away-from-home.md#how-it-will-be-measured) is for this case. |
| The Mac took the password, and Baton cannot connect afterwards | The Mac stops at the login window after the unlock, and no Baton runs until someone signs in there. | Sign in at the Mac. This is one of the things the restart test looks at. |

## What the route can reach

The device at home can reach every address in the range that you gave it, for everyone who can sign in to your Tailscale account. Two things keep that small. Protect the account that you sign in to Tailscale with, for example with two-step verification at that account. And limit what your phone may reach: Tailscale's access rules can allow the phone only the Mac's address, and the unlock needs only port 22 there. Baton's own connection needs more ports than that.

The password goes from the phone to the Mac inside the SSH connection, and the phone gives it only to a Mac that answers with a key that you saved, so a device that answers for the Mac's address gets nothing. Tailscale encrypts the connection as well, up to the device at home.
