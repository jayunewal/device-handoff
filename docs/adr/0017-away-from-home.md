# 0017: Away from home: a private network that the person runs carries the way, and Baton checks it

Status: proposed. This record and [the guide](../away-from-home.md) come first. The phone's check of the way to a saved Mac, and the saving of a Mac that has password sign-in turned off, follow in a later pull request. Checks A1 to A6 below are for the person at the devices. Nothing here has been tried on a Mac and a phone.

## Context

A person who is away from home wants two things from the Mac at home: to reach it while it runs, and to get it going again after it restarted. Both fail today for reasons that Baton's code does not cause.

- A home line behind carrier-grade NAT cannot take a connection from outside, so a device that is not at home can only reach the Mac through a relay. The relays that Baton uses by default are the iroh project's public ones, which are rate limited and meant for development and hobby use ([ADR 0003](0003-iroh-transport.md), [architecture.md](../architecture.md)).
- A Mac with FileVault runs nothing after a restart until the password of an account on it is given: not Baton, and not any network product. Since macOS 26 it takes that password over SSH when Remote Login is on, and the phone gives it there (`baton-unlock-ssh`, [security.md](../security.md)). That works only where the phone reaches the Mac's port 22, at an address, with no help from Baton's own connection.

What is known:

- Apple's page `apple_ssh_and_filevault(7)` says that when Remote Login is on, a password can be given over SSH before the disk is unlocked, that the usually configured authentication methods and shell access are not available then, that macOS drops the connection briefly while it mounts the data volume, and that the macOS version of OpenSSH keeps all of its configuration files, system wide and for each account, on the data volume (reported: quoted by a blog, the page itself was not read). The configuration of the running SSH server, and with it the setting that turns password sign-in off and the host keys in `/etc/ssh`, is on a volume that is locked at that time. So the door before the unlock probably does not read that setting, and probably does not answer with the key that the Mac shows when it runs (inferred: no source says either).
- Reports say that a Mac that was only on Wi-Fi could not be reached before sign-in until macOS 26.5 (reported, one author).
- A subnet router is a device on the home network that tells a Tailscale network that it carries a range of addresses. A phone on that network then reaches any address in the range, and the traffic is shown to the device it reaches as coming from the router device, unless that is turned off (documented, [Subnet routers](https://tailscale.com/kb/1019/subnets)). Approved routes are taken by an iPhone by itself (documented).
- A name such as `Jays-MBP.local` is announced on the local network and does not reach through a route (inferred from how such names work; no page of Tailscale was found that says it).

What is not known, and the checks below are for it:

- Whether the key that a Mac answers with before the disk is unlocked is the key that it shows when it runs, and whether it is the same at every restart.
- Whether the Mac signs the account in after the unlock, or stops at the login window, where no Baton runs.
- Whether a Mac that waits for its disk password answers at its address on Wi-Fi, and how long after the restart.
- How a connection of Baton's own behaves through the route.

## Decision

**Baton does not bring a network product, and has no code for one.** A person who wants the Mac from away runs a private network of their own, as ADR 0003 left it, and Baton works over any route that carries the Mac's address. The guide gives the steps for Tailscale because that is the route that was planned. Nothing in Baton depends on it, and Baton does not look at which one it is.

**The way in is a subnet router at home, and it is not the Mac.** A device that stays on, on the same network as the Mac, tells the private network that it carries the home range. The Mac needs no part in it. That is the only way a Mac that waits for its disk password can be reached from outside, since nothing runs on it that could join a network.

**The phone saves an address, not a name.** A name is announced on the local network and does not reach through a route, and whether a Mac that waits for its disk password announces one is not known. The guide has the person reserve a fixed address for the Mac in the router, and tells them not to use a Tailscale address, which the Mac does not have while it waits.

**The phone can check the way, and the check sends nothing.** From a saved Mac, **Check the way** asks the Mac for its key at the saved address, as the screen that adds a Mac does before it saves, and says how long the Mac took to answer and whether the key is one of the saved ones. It never sends the password, so it can be used on any network, as often as the person likes, with a Mac that runs and with one that waits for its disk password. When it fails, it names the likely reasons and points to the guide. Until that check is in the app, the screen that adds a Mac does the same.

**A running Mac whose SSH takes keys only can be saved.** The phone tries the password on the running Mac before it saves one, and now refuses to save a Mac that does not take a password. The notes above suggest that the door before the unlock does not read that setting, so the refusal can keep out a Mac that would work. The phone says that the password could not be tried, as it does when the Mac does not answer, and saves it when the person says so.

**The key of the Mac before its disk is unlocked is learnt only in a step that the person starts.** The phone gives the password only to a key that was saved, and that stays. If check A3 shows that the Mac answers with a key of its own, the phone will learn that key in a step that the person starts, on the home network, with the Mac in front of them at its disk password screen, and the step will say that whatever answers in that minute is trusted from then on. A key that was not learnt in that step is refused, as it is now. That step is not built. It waits for A3, and a reviewer reads it before it is built, because it is the one place where the phone would trust a key that the person could not compare with anything.

## Consequences

- The device at home becomes a way into the home network. It can reach every address of the range that it carries, for everyone who can sign in to the account of the private network. The guide says to protect that account and to limit the phone to what it needs.
- Nothing wakes a Mac that sleeps. Keeping the Mac awake while it charges is a later change. The device at home could send the standard wake message for the person, which a route does not carry, and that is not designed here.
- A connection of Baton's own through the route to the Mac's home address is reported as "over your local network", since the address is a private one, and the words do not say that it went through the route.
- If the Mac stops at the login window after the unlock, Baton is not running and a person has to sign in. How to get past that is a separate decision.

## How it will be measured

On a Mac and an iPhone, with the guide set up and the builds, models and networks recorded. Each check is done by the person at the devices.

- **A1. The way, at home.** The phone on the home Wi-Fi, Tailscale off. **Add a Mac**, the Mac's fixed address, **Check the Mac**. The key appears, and it is the one that `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` prints at the Mac.
- **A2. The way, away.** Wi-Fi off on the phone, mobile data on, Tailscale on. The same check with the same address. The same key appears. How many seconds does it take? If it fails, which of the reasons in the guide was it?
- **A3. The Mac at its disk password screen.** Restart the Mac and leave the disk password screen up. From the phone on the home Wi-Fi, Tailscale off, check the Mac. Does it answer, and how long after the restart? With which key: the one of A1, or another? If another, restart again and check again: is it the same one both times?
- **A4. The unlock, at home.** At the same screen, tap **Unlock** on the saved Mac and look at the phone for the face. Does the Mac take the password? How long until it shows the login window or the desktop? Does it sign the account in by itself, or wait at the login window? Is Baton running afterwards?
- **A5. The unlock, away.** A3 and A4 again with mobile data and Tailscale on.
- **A6. A Mac that sleeps.** Put the Mac to sleep, and check it from the phone at home and away. It should not answer, and the phone should say that the Mac did not answer and not that something is wrong with the key.

## Open points

- **The key before the unlock (A3).** If it is the key of the running Mac, there is nothing to build. If it is a key of its own that is the same at every restart, the step that learns it is built and reviewed. If it changes at every restart, no saved key can protect the unlock, and this record is replaced.
- **After the unlock (A4).** A Mac that waits at the login window needs a separate decision.
- **Waking a Mac.** A device at home could send the wake message, and the phone could ask it to. Not designed.
- **The words for a route.** A connection through the route is called a local one. Whether that misleads depends on what the checks show.
