# Baton on Windows: what is possible when the machine is locked

Research date: 2026-10-14 (Windows 11 26H2 reached general availability on 2026-09-29).
Scope: Windows only (Apple and Linux/Android are covered by other researchers).

Requirement under test: handing work off must keep working when the devices are locked, because the person will not leave the previous device unlocked.

## Labels and limits (read first)

Every claim carries one of these labels and a source URL:

- [C] confirmed in official documentation (Microsoft Learn, support.microsoft.com, SDK reference pages).
- [S] confirmed by source code I read directly, or by a credible report (vendor knowledge base, security write-up). [S-weak] marks a single forum post, community Q&A answer, blog or tech-press article.
- [I] inferred by me from other facts. No source states it directly.
- Unknown: I looked and found no evidence either way.

Limits of this research:

- The web-search budget shared by the research agents ran out before the last gap-filling searches (Windows.Graphics.Capture while locked, clipboard while locked, same-user RDP versus the console lock). Those stay Unknown. A follow-up run with a larger search budget is needed to try again.
- Web pages were read through a summarising fetch tool. Quoted sentences are as that tool returned them: treat them as accurate, not byte-verified.
- Nothing was run on real Windows hardware. Section (c) lists what must be tested, with exact steps.
- Conflicts and gaps are collected in Appendix B. Facts I used from general knowledge without verifying them here are listed in Appendix C.

---

## (a) Summary table

Definitions. "Locked" = user signed in, session locked (Win+L, idle lock, Dynamic Lock), machine awake (S0), screen on or off. "Asleep" = machine entered sleep through the power button, lid, idle timeout or Start menu; the power model is either Modern Standby (S0 low power idle) or classic S3, fixed by hardware and firmware. "Before first sign-in" = booted, nobody signed in yet (sign-in screen), for example after a restart or power-on. "Agent" = a per-user Baton background process started at sign-in.

| Capability | Locked (awake) | Asleep: Modern Standby | Asleep: S3 | Before first sign-in |
|---|---|---|---|---|
| Background agent alive | **Works** - lock only switches the input desktop inside the same session; processes keep running [C][S][I] (Q1) | **No** - the Desktop Activity Moderator (DAM) suspends all threads of session 1+ apps and throttles session-0 services (24H2+: may suspend them) [C] (Q1) | **No** - system suspended; only hardware wake (Wake-on-LAN) [C] (Q1) | **Partly** - a per-user agent cannot run (Run key, Startup and logon tasks fire at logon [C]); a boot-start LocalSystem service can, only if the volume is unlocked (not with BitLocker TPM+PIN) [C][I] (Q2) |
| Network reachable | **Works** - lock does not touch networking; Sunshine, RustDesk and Chrome Remote Desktop host services serve clients while the console is locked [S][I] (Q8) | **Partly** - NIC in low-power/WoL-pattern mode; network is quiesced on battery unless Remote Desktop or background tasks need it; nothing can answer until resume [C] (Q1) | **Partly** - only a Wake-on-LAN magic packet or pattern wakes the machine; the agent must then re-announce itself [C][I] (Q1) | **Partly** - the OS network is normally up after boot [I], but only a service can listen; nothing with TPM+PIN [C][I] (Q2) |
| Clipboard text | **Unknown** - the clipboard belongs to the session's WinSta0, which a locked session keeps [C]; no source tests set/listen while locked; likely works [I] (Q3) | **No** - agent suspended [C] | **No** - suspended [C] | **No** - no user clipboard exists; a session-0 service has a different window station [C] (Q3) |
| Open link / file | **Partly** - the agent can probably launch the handler [I], but the window opens on the hidden Default desktop until unlock; queue and open on unlock, notify with a toast (Reminder/Alarm/IncomingCall-class toasts can show above the lock screen if the setting is on [C][S-weak]) (Q4) | **No** - agent suspended [C] | **No** - suspended [C] | **No** - no user session to open it in [I] (Q4) |
| Read files | **Works** for local files [I]; cloud-only OneDrive placeholders (they need the sync engine [C]) and EFS files are both Unknown (Q5) | **No** - agent suspended [C] | **No** - suspended [C] | **Partly** - a SYSTEM service can read what the ACLs allow if the volume is unlocked [I]; no OneDrive hydration, no user DPAPI/EFS context [I] (Q5) |
| Window capture | **No** - DXGI Desktop Duplication returns E_ACCESSDENIED on the secure desktop unless LOCAL_SYSTEM [C]; automation vendors report a paused session, black or stale images [S]; Windows.Graphics.Capture untested [Unknown]. A SYSTEM helper can capture the lock screen itself, not the user's windows [S] (Q6) | **No** - agent suspended [C] | **No** - suspended [C] | **Partly** - only the sign-in screen, through a SYSTEM helper attached to the input desktop [S] (Q6) |
| Input injection | **No** for the user's apps ("system ignores all keypresses and mouse actions inside of the locked session" [S-weak]; desktop and UIPI barriers [C]); **Partly** at the lock screen through a SYSTEM helper, which means typing credentials [S] (Q7) | **No** - suspended [C] | **No** - suspended [C] | **Partly** - SYSTEM helper only [S] (Q7) |
| Full-session remote view | **No** on the console; **Partly** only by moving the session off the console into an RDP session (Chrome Remote Desktop "curtain" pattern: loopback RDP, Windows Pro or higher, RDP on, NLA off) [S]; Windows Home cannot host RDP [C] (Q8) | **No** - suspended [C] | **No** - suspended [C] | **Partly** - sign-in screen only, helper or RDP to the logon screen on Pro+ [S][I] (Q8) |

### Notes on reading the table

1. "Locked" is only as long as the machine stays awake. A laptop with Modern Standby that is locked and left alone goes to sleep after the user's sleep timeout (user-configurable; OEM defaults vary; I did not verify Windows defaults). After that the whole "Asleep" column applies. For the owner's scenario (walk away from device A, keep working on device B), device A being asleep is likely the common case on laptops, so Baton needs a store-and-forward path (relay or sender queue) for sleeping devices [I].
2. DAM is documented as disengaged "when the screen is on"; the Modern Standby overview lists the entry triggers as power button, lid close, idle timeout and Start-menu Sleep, and does not list the display-off timeout alone [C]. Whether "locked, screen off, not yet asleep" counts as Modern Standby is therefore not documented. Treated as awake here [I]; must be tested (T02, T06).
3. On Modern Standby the docs say: "Windows prevents desktop applications from running during any part of modern standby after completing the DAM phase." A Wake-on-LAN style packet can wake the SoC, but nothing in the docs says desktop apps resume without the screen turning on [C][I]. Unknown whether a network wake with the display off un-suspends them (T06, T07).

### What this means for the owner's requirement [I]

- Receiving hand-offs (link, clipboard text, file) on a device that is locked but awake: a per-user agent should be able to do it, because the session, the network and the clipboard belong to the session, not to the input desktop. Two pieces are untested (clipboard and launching a handler while locked; T03, T04).
- Receiving on a device that is asleep: not possible on Modern Standby, and only through Wake-on-LAN on S3, so design for "queue at the relay or sender, deliver on resume" and do not promise delivery to a sleeping laptop.
- Streaming windows from a device that is locked: no documented API captures or controls a user's windows while the session is locked. The options are (1) require the source device unlocked during a stream, which conflicts with the owner's wish; (2) move the session into an RDP session (heavy, Pro-only, weakens host security); (3) a virtual display (behaviour while locked is Unknown); (4) finish the hand-off and then lock the source with `LockWorkStation()`, which is documented and callable from the user-session agent. Option 4 satisfies "do not leave my previous device unlocked" for the first releases.
- Anything that touches the lock screen itself (capturing it, typing a PIN or password) needs a LocalSystem helper and should be an explicit opt-in "advanced" feature, never default. Remote unlock should not be built.

---

## (b) Answers to the 12 questions

### 1. Lock-state basics, sleep models, keep-awake and policies

1.1 What "locked" is
- [C] Each session has its own interactive window station "WinSta0" with three standard desktops (Winlogon, screen saver, interactive/Default). One desktop is the input desktop at a time. The system switches to the Winlogon desktop at logon, on Ctrl+Alt+Del and when the UAC dialog is open. "The Winlogon desktop's security descriptor allows access to a very restricted set of accounts, including the LocalSystem account. Applications generally do not carry any of these accounts' SIDs in their tokens and therefore cannot access the Winlogon desktop or switch to a different desktop while the Winlogon desktop is active." (https://learn.microsoft.com/en-us/windows/win32/winstation/desktops ; per-session WinSta0: https://learn.microsoft.com/en-us/windows/win32/termserv/terminal-services-sessions)
- [S] RustDesk recognises the lock screen by finding LogonUI.exe running in the user's session, and its comments note that "a LogonUI.exe can outlive the unlock". The Apollo wiki describes the same arrangement. So the lock UI is a process in the same session on the Winlogon desktop. (https://raw.githubusercontent.com/rustdesk/rustdesk/master/src/platform/windows.rs ; https://github.com/ClassicOldSong/Apollo/wiki/Auto-lock-unlock-Windows)
- [I] Consequence: the user's processes keep running during a lock; only the input desktop changes. Indirect support: DAM "is disengaged and does not impact any processes" while the screen is on [C], and Chromium and Electron deliver lock and unlock events to live processes [S] (sources in section (d), D.2).

1.2 Learning and starting a lock
- [C] There is no query function: "There is no function you can call to determine whether the workstation is locked." Microsoft recommends `WTSRegisterSessionNotification`. (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-lockworkstation)
- [C] `LockWorkStation` works like Ctrl+Alt+Del then Lock, is asynchronous, requires a caller on the interactive desktop with a logged-on user, and can be denied by the GINA/credential provider. So Baton can lock its own device from the user-session agent, not from a session-0 service. (same page; the "not from a service" reading is [I])
- Detection APIs are in section (d).

1.3 Modern Standby (S0 low power idle)
- [C] Entry starts when the user causes sleep: power button, lid close, idle timeout, or Sleep in the Start menu. In this model the PC "stay[s] connected to the network while in a low power mode"; the NIC is in "low-power, protocol offload, and WoL patterns mode" unless a system service or Store background task needs the network; wake sources include user input, network device interrupts and network packets. (https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/modern-standby)
- [C] Adaptive Connected Standby (ACS) is the default since Windows 10 2004; on battery "Windows provides Internet connectivity to apps that require it and otherwise prevents networking activity during sleep"; the network is quiesced unless Remote Desktop is enabled or UWP background tasks need it; the ACS mode is fixed for a sleep session. (https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/modern-standby-network-connectivity)
- [C] DAM: processes created in session 1+ get all threads suspended (memory kept); session-0 services are throttled "to no more than one second of activity every 30 seconds"; "As of 24H2, additional session-0 services may be suspended, and session-0 service throttling may be stopped to decrease battery drain." "When the screen is on, the DAM is disengaged and does not impact any processes on the system." "After a few seconds delay, DAM suspends the process." Opted-in processes receive WM_POWERBROADCAST suspension notices through `RegisterSuspendResumeNotification` or `PowerRegisterSuspendResumeNotification`. The DAM driver exists only on client SKUs that support connected standby. (https://learn.microsoft.com/en-us/windows/compatibility/desktop-activity-moderator ; https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/prepare-software-for-modern-standby)
- [C] "Windows prevents desktop applications from running during any part of modern standby after completing the DAM phase." "Power requests will block the NoCS phase indefinitely on AC power, and for up to 5 minutes on DC power. Audio and mobile hotspots are allowed to run indefinitely." The pages I read do not define "NoCS", so I do not interpret it; test T06 measures the real effect. (prepare-software-for-modern-standby page above)
- [C] Timer-triggered Microsoft Store background tasks run only on AC power, at most every 15 minutes. (https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/integrating-apps-with-modern-standby) [I] Store background tasks look like the sanctioned way to get code to run during Modern Standby; whether a sideloaded MSIX component outside the Store could use them is not researched.
- [C] Microsoft expects Wake-on-LAN "to work the same way on a Modern Standby system and an S3 system". This is an expectation, not a guarantee, and WoWLAN on Wi-Fi is not covered. (https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/modern-standby-vs-s3)
- [C] The power model cannot be switched in firmware: "Switching between S3 and Modern Standby cannot be done by changing a setting in the BIOS" (modern-standby page above). The DAM page says the driver loads at boot if the AOAC field of SYSTEM_POWER_CAPABILITIES (from `CallNtPowerInformation`) is TRUE, which is a runtime way to detect the model.

1.4 S3, hibernate, Fast Startup, shutdown
- [C] In S3 the system is suspended; wake needs a hardware event such as Wake-on-LAN (modern-standby-vs-s3 page above).
- [C] BitLocker: resume from sleep needs no PIN; resume from hibernation requires the PIN or startup key when TPM+PIN is used. (https://learn.microsoft.com/windows/security/operating-system-security/data-protection/bitlocker/countermeasures)
- [C] Fast Startup: a shutdown logs off all sessions and hibernates the kernel session; a restart is a full boot; `shutdown /s /hybrid` opts into the hybrid shutdown. After "shut down, then power on" the user must sign in again, so a per-user agent only starts after sign-in. (https://learn.microsoft.com/en-us/troubleshoot/windows-client/setup-upgrade-and-drivers/fast-startup-causes-system-hibernation-shutdown-fail ; https://learn.microsoft.com/en-us/windows-hardware/test/weg/delivering-a-great-startup-and-shutdown-experience)

1.5 Keep-awake
- [C] `SetThreadExecutionState` cannot prevent user-initiated sleep (lid, power button, Start-menu Sleep), does not stop the screen saver, and away mode works only on S3. (https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate)
- [C] `PowerSetRequest`: on Modern Standby on DC power, system-required and execution-required requests are terminated 5 minutes after the sleep timeout expires; all power requests (except AwayMode on S3) are terminated when user-initiated sleep begins; `PowerRequestExecutionRequired` exists. (https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-powersetrequest)
- [I] A "stay awake while a hand-off session is active" option is feasible only on AC power and only until the user closes the lid or presses sleep. It cannot make a sleeping or battery-powered Modern Standby laptop reachable.

1.6 Policies seen in the docs
- [C] "Disable or enable software Secure Attention Sequence" gates `SendSAS` (Q7). The ARSO sign-in/lock policy is described in Q2. "Configure dynamic lock factors" is described in Q12. Power policies for network connectivity in standby and inactivity locks were not researched (Appendix C).

### 2. Before sign-in: services, scheduled tasks, BitLocker, service reading user profiles

2.1 What starts before a user signs in
- [C] Run and RunOnce keys run at logon, may be delayed, and are ignored in Safe Mode by default. (https://learn.microsoft.com/en-us/windows/desktop/SetupApi/run-and-runonce-registry-keys) The Startup folder has logon-time semantics too [I]; I did not source it separately.
- [C] The Task Scheduler LogonTrigger fires when a user logs on (https://learn.microsoft.com/en-ie/windows/win32/taskschd/logontrigger). The BootTrigger fires "when the Task Scheduler service starts"; only Administrators can create boot triggers; the page does not state the account (https://learn.microsoft.com/en-us/windows/win32/taskschd/boottrigger). Whether a SYSTEM-principal task runs in session 0 is general knowledge, not verified here (Appendix C).
- [C] Services run in session 0 and cannot interact with the user; Microsoft's recommended alternatives are `WTSSendMessage` or a helper launched with `CreateProcessAsUser`. (https://learn.microsoft.com/en-us/windows/win32/services/interactive-services)
- [C] LocalSystem has extensive local privileges, no password, and presents the computer's credentials on the network. (https://learn.microsoft.com/en-us/windows/win32/services/localsystem-account)
- [I] A boot-start auto service is the more predictable way to run before sign-in than a boot-trigger task: ordering, restart-on-failure and `SERVICE_CONTROL_SESSIONCHANGE` callbacks all exist for services (Q10).

2.2 BitLocker decides whether anything runs at all
- [C] TPM-only unlocks without user interaction, so the machine reaches the sign-in screen and boot-start services run. TPM+PIN needs the PIN at the pre-boot screen, so no code runs until someone types it. (https://learn.microsoft.com/windows/security/operating-system-security/data-protection/bitlocker/countermeasures)
- [C] Network Unlock (domain-joined, WDS and DHCP, wired) can auto-unlock a TPM+PIN machine on the corporate network. (https://learn.microsoft.com/en-us/windows/security/operating-system-security/data-protection/bitlocker/network-unlock)
- [I] A reachable-before-sign-in mode therefore cannot work on TPM+PIN laptops outside the corporate network. Whether consumer "device encryption" is TPM-only was not sourced here (Appendix C).

2.3 Winlogon Automatic Restart Sign-On (ARSO)
- [C] After a Windows Update-initiated restart, the last interactive user is signed in automatically and the session is locked (so lock-screen apps can run and updates that need a sign-in can finish). It is limited to system-initiated restarts; client SKUs have an opt-out; the Quick Overview says ARSO "Can only be enabled if BitLocker is enabled". (https://learn.microsoft.com/en-us/windows-server/security/windows-authentication/winlogon-automatic-restart-sign-on-arso)
- [I] After a patch restart a device can be "locked but running the user's session" without anyone typing a password, so a per-user agent may start in a locked session (T17). The agent must read its initial lock state at start (section (d)), not assume "unlocked".

2.4 A service reading user profiles
- [I] A LocalSystem service can open files under `C:\Users\<name>` if the ACLs grant SYSTEM, and the volume is unlocked after boot. It has no user token, so no OneDrive client (a per-user process), no mapped drives, and no access to the user's DPAPI or EFS keys. Not sourced; EFS and DPAPI behaviour are Unknown (T11, T08).
- [C] When a user is signed in, `WTSQueryUserToken` (caller must be LocalSystem with SE_TCB) returns that user's token so a service can start a helper as the user. (https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsqueryusertoken)
- [C] "A service will only be notified of a user logon if it is fully loaded before the logon attempt." (https://learn.microsoft.com/en-us/windows/win32/api/winsvc/nc-winsvc-lphandler_function_ex)

2.5 Verdict [I]
Pre-sign-in reachability is a separate, opt-in "always on" mode: per-machine install, a LocalSystem service, an inbound firewall rule, no support on TPM+PIN machines, and a large security surface. Recommend not building it in the first release.

### 3. Clipboard while locked

- [C] A window station contains the clipboard and the atom table (https://learn.microsoft.com/en-us/windows/win32/winstation/window-stations). Each session has its own WinSta0, and a lock switches desktops inside that window station (desktops page in 1.1), so the locked user's clipboard still exists. [I] A process whose thread sits on the Default desktop keeps access to it.
- [C] `OpenClipboard` fails if another window has the clipboard open; `AddClipboardFormatListener` posts `WM_CLIPBOARDUPDATE`; the docs say nothing about desktops, sessions or the locked state. (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-addclipboardformatlistener ; https://learn.microsoft.com/en-us/windows/win32/dataxchg/about-the-clipboard)
- Unknown: I found no Microsoft statement, open-source comment or credible report that tests setting the clipboard or receiving clipboard updates while the workstation is locked. The follow-up searches hit the search budget. Test T03.
- [C][I] A session-0 service has a different, non-interactive window station and cannot write the user's clipboard; use a helper in the user session (interactive-services page in Q2). Before sign-in there is no user clipboard to write into.
- [I] Design: treat "set clipboard while locked" as best effort. Try; read back; if `OpenClipboard` fails or the read-back differs, queue and apply on `WTS_SESSION_UNLOCK`; tell the sender which happened.

### 4. Opening URLs and files, toasts on the lock screen

- [I] A launch from the user-session agent (for example `ShellExecuteW`) probably starts the default handler, but any window it creates is on the Default desktop, which is not the input desktop while locked, so the user sees it only after unlocking. Whether the launch itself succeeds, and whether it hangs when it hands off to an already-running browser, is Unknown (T04). Related risk: a GUI test vendor reports window messages "pause or hang" in a locked session [S] (https://qatools.knowledgebase.qt.io/squish/troubleshoot/lack-of-display-on-windows/), so never block the agent on a launch; use a watchdog.
- [I] Best behaviour: queue links and open them on `WTS_SESSION_UNLOCK`, and show a notification while locked.
- [C] Toast scenarios are Default, Reminder, Alarm and IncomingCall; the last three stay on screen until dismissed. The page says nothing about the lock screen. (https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/toast-schema)
- [C] Windows 11 has two user settings: "Show notifications on the lock screen" and "Show reminders and incoming VoIP calls on the lock screen" (default on). (https://eus.prod.support.services.microsoft.com/en-gb/topic/8942c744-6198-fe56-4639-34320cf9444e)
- [S-weak] The second setting maps to registry value `NOC_GLOBAL_SETTING_ALLOW_CRITICAL_TOASTS_ABOVE_LOCK` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings`. (https://www.elevenforum.com/t/enable-or-disable-notifications-for-reminders-and-voip-calls-on-lock-screen-in-windows-11.47882/)
- [I] Reminder, Alarm or IncomingCall scenario toasts are the likeliest to appear above the lock screen; clicking one cannot reveal Baton's window until sign-in. Whether an unpackaged, installer-distributed app can raise such toasts on the lock screen is Unknown (T10); the prerequisites for toasts from unpackaged apps are in Appendix C.

### 5. Reading user files while locked, and before sign-in

- [I] File access depends on the process token, not on the lock, so a user-session agent should read local files normally while locked. No source says otherwise, and none confirms it.
- [C] Windows Information Protection (deprecated July 2022) revoked keys while the device was locked (`ProtectedAccessSuspending`). It only matters on managed machines still using WIP. (https://learn.microsoft.com/en-us/uwp/api/windows.security.enterprisedata.protectionpolicymanager.protectedaccesssuspending?view=winrt-22621)
- [C] OneDrive Files On-Demand placeholders are "only available if the sync service is available"; `cldflt.sys` proxies reads to the sync engine. (https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine) [I] The OneDrive client is a per-user process, so cloud-only files can be served only when it is running, and it is suspended by DAM in Modern Standby. Behaviour while locked: Unknown (T11). Known Folder Move (Desktop/Documents/Pictures redirected into OneDrive) makes this common, [I] so it matters.
- Unknown: EFS-encrypted files while locked. No source found. [I] EFS keys belong to the user's profile, so a SYSTEM service before sign-in should not be able to read the user's EFS files.
- [I] Before sign-in: a SYSTEM service can read files whose ACLs allow SYSTEM on an unlocked volume, with none of the user-context features above (see Q2.4).

### 6. Screen and window capture while locked

6.1 Documented limits
- [C] `IDXGIOutput1::DuplicateOutput` returns `E_ACCESSDENIED` "if the application does not have access privilege to the current desktop image. For example, only an application that runs at LOCAL_SYSTEM can access the secure desktop." Other documented failures: `DXGI_ERROR_SESSION_DISCONNECTED`, a limit on the number of processes duplicating one output (four per session), `DXGI_ERROR_UNSUPPORTED` for non-DWM modes. (https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput)
- [C] `AcquireNextFrame` returns `DXGI_ERROR_ACCESS_LOST` when the desktop switches or the mode changes; the app must recreate the duplication. (https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe)
- [S] GStreamer's `d3d11desktopdupsrc` merge request confirms that the UAC secure desktop makes `DuplicateOutput` return `E_ACCESSDENIED`; the fix is `SetThreadDesktop` to the input desktop and running as LOCAL_SYSTEM. (https://gitlab.freedesktop.org/gstreamer/gst-plugins-bad/-/merge_requests/2209)

6.2 How shipping products do it
- [S] Sunshine: `syncThreadDesktop()` calls `OpenInputDesktop(DF_ALLOWOTHERACCOUNTHOOK, FALSE, GENERIC_ALL)` then `SetThreadDesktop` before each `DuplicateOutput`; on `ACCESS_LOST`, `ACCESS_DENIED` or `WAIT_ABANDONED` it re-initialises. Its Windows service launches the app into the console session with a duplicated LocalSystem token (details in Q7). (https://raw.githubusercontent.com/LizardByte/Sunshine/master/src/platform/windows/display_base.cpp ; https://raw.githubusercontent.com/LizardByte/Sunshine/master/src/platform/windows/misc.cpp ; https://raw.githubusercontent.com/LizardByte/Sunshine/master/tools/sunshinesvc.cpp)
- [S] RustDesk: `desktop_changed()` makes the capture loop bail out and relaunch; it falls back from DXGI to GDI; it has UAC elevation checks. (https://raw.githubusercontent.com/rustdesk/rustdesk/master/src/server/video_service.rs ; https://raw.githubusercontent.com/rustdesk/rustdesk/master/libs/scrap/src/dxgi/mod.rs)
- [S] Looking Glass wiki (project docs): the logon screen, UAC, Ctrl+Alt+Del and the lock screen are on the Secure Desktop and capture is intentionally limited; an old note was marked deprecated on 2020-08-12; a later revision says a helper service started at login with elevated privileges lets everything be captured "only the Login Screen cannot be captured". This is partly at odds with Sunshine and RustDesk, see Appendix B. (https://looking-glass.io/wiki/Special:MobileDiff/607)

6.3 The user's own windows while locked
- [S] WinAppDriver (Microsoft): "While the machine is locked, the interactive session is paused." (https://raw.githubusercontent.com/microsoft/WinAppDriver/master/./Docs/RunningOnRemoteMachine.md)
- [S] Squish (Qt) KB: a locked desktop gives black, white or distorted screenshots; window messages pause or hang. (https://qatools.knowledgebase.qt.io/squish/troubleshoot/lack-of-display-on-windows/)
- [S-weak] `PrintWindow` on a window of an inactive desktop returns stale images (https://windows-hexerror.linestarve.com/q/so46432965-PrintWindow-on-inactive-desktop-in-Windows-10); in a disconnected RDP session it returns black (https://www.autoitscript.com/forum/topic/134261-user32dll-printwindow-in-a-disconnected-remote-desktop-session/).
- [S] Microsoft Q&A (reproduced by Microsoft staff): DXGI Desktop Duplication fails after an RDP reconnect on a Windows 10 client (0x80070057), not on Server. (https://learn.microsoft.com/en-us/answers/questions/1068807/desktop-duplication-api-doesnt-work-over-remote-de)
- [S-weak] Conflict: an OBS forum thread (2016) says "The OS stops rendering the UI entirely when the screen is locked" (https://obsproject.com/forum/threads/black-screen-during-locked-screen-rdp-recording.53319/latest); a 2022 post says on Windows 10 "obs manages to record the lock screen" (https://obsproject.com/forum/threads/how-could-i-record-without-needing-to-be-on-the-desktop-like-on-the-lock-screen.78353/latest). Unresolved.
- Unknown: Windows.Graphics.Capture (WGC), DirectComposition surfaces, `BitBlt` and `PrintWindow(PW_RENDERFULLCONTENT)` for a window while the session is locked. The Microsoft screen-capture page is silent about locked sessions and the secure desktop (https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture). Test T05 is the highest-value test in this report.
- [C] `WDA_EXCLUDEFROMCAPTURE` (Windows 10 2004+) works only when DWM is composing. (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)

6.4 Reading of the evidence [I]
Everything in the shipping products points the same way: capturing the lock screen, sign-in screen and UAC needs a LocalSystem process attached to the input desktop, and none of them claim to stream an app window while the session is locked. Until T05 shows that a standard API still delivers live frames of a user window during a lock, plan on "no".

### 7. Input injection while locked

7.1 Documented limits
- [C] `SendInput` is subject to UIPI and fails silently when blocked (neither the return value nor `GetLastError` reports a UIPI block). (https://learn.microsoft.com/tr-tr/windows/win32/api/winuser/nf-winuser-sendinput) `BlockInput` can only be released by the blocking thread, and Ctrl+Alt+Del unblocks it. (https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-blockinput?redirectedfrom=MSDN)
- [C] `SendSAS` (Ctrl+Alt+Del): "an application must either be running as a service or have the uiAccess attribute ... set to "true""; a non-service caller must run as the current user or LocalSystem and UAC must be on; uiAccess applications must be Authenticode-signed and reside in a protected location (\Program Files\ or \windows\system32\); the policy "Disable or enable software Secure Attention Sequence" applies. (https://learn.microsoft.com/en-us/windows/win32/api/sas/nf-sas-sendsas)
- [C] UI Automation: UIAccess cannot reach UI at system integrity level, such as UAC and the logon UI. (https://learn.microsoft.com/en-gb/windows/win32/winauto/uiauto-securityoverview)
- [S-weak] A vendor statement: "system ignores all keypresses and mouse actions inside of the locked session". (https://www.robotask.com/forum/viewtopic.php?p=3242) AutoIt: `Send()` fails while locked. (https://autoitscript.com/forum/topic/155441-processing-keystrokes-behind-a-locked-screen)

7.2 How products reach the lock screen
- [S] Chrome Remote Desktop, RustDesk and Sunshine each run a LocalSystem service that places a helper in the console session, attached to the input desktop. Baton does not plan to build such a helper (locked-devices.md, sections 5.5 and 5.8), so the code-level detail is not reproduced in this copy. The sources are the public source code of the three projects.

7.3 Security and policy [I unless noted]
- A SYSTEM helper attached to the Winlogon desktop that can press keys is a typing-credentials-for-the-user machine. A bug in it, or in the IPC to it, is local privilege escalation.
- Real-world precedent that session scoping goes wrong: two RustDesk advisories (peer with only the FileTransfer scope could inject input) [S] (https://app.opencve.io/cve/CVE-2026-58056 ; https://app.opencve.io/cve/CVE-2026-57850).
- If built at all: separate signed binary; installed only on explicit opt-in; named pipe with a DACL for the console user's SID only and a check of the client binary's signature; off by default and visibly indicated when active; no "type my password" feature; log every use; per-session consent.
- Do not copy the tscon "auto lock/unlock" scripts in Q8: they can leave the machine unlocked [S].

### 8. RDP, tscon, shadowing, loopback RDP, curtain modes, privacy modes

8.1 RDP on client Windows
- [C] The PC being connected to "needs to be running a Pro edition of Windows" (https://support.microsoft.com/en-us/windows/how-to-use-remote-desktop-5fe128d5-8fb1-7a23-3b8a-41e636865e8c). A single fetch of the Learn page also summarised "Home cannot serve as a host" (https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/remotepc/remote-desktop-allow-access). So Home cannot host RDP, which also rules out RDP-based redirection on Home.
- [S-weak] Community Q&A (2021, volunteer answer, no Microsoft links): on Windows 10 Pro/Enterprise/Home an incoming RDP session logs off the local user and a local sign-in ends the RDP session, "by design"; Windows Server converts the console session to a remote session for the same account (https://learn.microsoft.com/en-us/answers/questions/383792/rdp-session-local-user-logs-off-when-the-remote-co). Another thread: "A remote session takes over the machine's console" and "With RDP you start an own session on the remote host, shown only in your RDP client" (https://learn.microsoft.com/en-us/answers/questions/382146/rdp-how-to-show-black-screen-during-session).
- [S] WinAppDriver docs: closing the Remote Desktop window locks the computer (RunningOnRemoteMachine.md, URL in Q6.3). This is not the same as the Q&A's "logs off". No official Microsoft statement found for same-user versus different-user behaviour or for the state of the local console after an RDP disconnect. Test T14.
- [C] `tscon` "connects to another session"; needs Full Control or Connect permission; a password is required if you do not own the target session; "You can't connect to the console session."; `/dest:<sessionname>` names the current session, which disconnects. (https://learn.microsoft.com/is-is/windows-server/administration/windows-commands/tscon)

8.2 Loopback RDP and Chrome Remote Desktop curtain mode
- [S] The CRD Windows host has two desktop-session classes: `ConsoleSession` ("attaches to the host's physical console") and `RdpSession` ("attaches to virtual RDP console"). The RDP one uses `RdpClient`, hosting the RDP ActiveX control, to open a loopback connection to 127.0.0.2 (the code comment: "127.0.0.1 is explicitly blocked by the RDP ActiveX control"), with a requested resolution (default 1280x768) and a terminal id. (https://raw.githubusercontent.com/chromium/chromium/main/remoting/host/desktop_session_win.cc ; https://raw.githubusercontent.com/chromium/chromium/main/remoting/host/win/rdp_client.cc ; https://raw.githubusercontent.com/chromium/chromium/main/remoting/host/win/rdp_client_window.cc)
- [S] `VerifyRdpSettings()` requires `fDenyTSConnections` not equal 1, NLA disabled and a security layer other than TLS-only. (desktop_session_win.cc above)
- [S] Curtain check: `return WTSGetActiveConsoleSessionId() != session_id;`, so the host is "curtained" when its session is not attached to the local console; the client disconnects if the session re-attaches to the console. (https://raw.githubusercontent.com/chromium/chromium/main/remoting/host/curtain_mode_win.cc)
- [S] Google's admin page: Windows Professional/Ultimate/Enterprise/Server; `RemoteAccessHostRequireCurtain=1`, `fDenyTSConnections=0`, `UserAuthentication=0`, and on Windows 10 also `SecurityLayer=1`. (https://support.google.com/chrome/a/answer/2799701)
- Not established: whether the CRD RDP session re-attaches the user's existing interactive session or creates a new one. I did not trace how the loopback connection authenticates. Test T15 settles it with `mstsc /v:127.0.0.2`.
- [I] This is the only production pattern found that keeps the physical console locked while a remote viewer uses the live session, and its price is high: Pro or higher, RDP enabled, NLA off, and a session move that ends if someone logs in at the console.

8.3 Shadowing
- [S] `mstsc /shadow:<id>` (and `/noConsentPrompt`) against a locked session or a UAC prompt gives a black shadow window with a pause symbol. (https://woshub.com/rdp-session-shadow-to-windows-10-user ; https://swarm.ptsecurity.com/remote-desktop-services-shadowing/) So shadowing cannot watch a locked console.

8.4 Privacy modes and their weaknesses
- [S] RustDesk has four implementations: exclude-from-capture (`SetWindowDisplayAffinity`), Magnification (`win_mag`), a virtual display (IDD), and a topmost black window with `WH_KEYBOARD_LL`/`WH_MOUSE_LL` hooks. When UAC runs, the code logs "In privacy mode, but show UAC prompt window for now". (https://raw.githubusercontent.com/rustdesk/rustdesk/master/src/privacy_mode.rs ; https://raw.githubusercontent.com/rustdesk/rustdesk/master/src/server/video_service.rs)
- [S] TeamViewer's black-screen mode shows a security image and disables local input; Ctrl+Alt+Del removes it; interplay with lock and UAC is not addressed. (https://www.teamviewer.com/en/global/support/knowledge-base/teamviewer-remote/security/teamviewer-black-screen/)
- Unknown: AnyDesk privacy-mode mechanism (the vendor page extracted empty); Parsec privacy mode (nothing found).
- [S-weak] Quick Assist shows a blank panel for UAC prompts because of the secure desktop. (https://learn.microsoft.com/en-ie/answers/questions/238459/uac-prompt-is-blocked-when-i-use-quick-assist)
- [I] Hook-and-overlay privacy modes cannot cover the Winlogon desktop, because hooks and windows belong to the Default desktop; only the RDP-redirect model gives a physical-console curtain.

8.5 Security records worth knowing
- [S] CVE-2019-9510: RDP with NLA auto-reconnect restores the remote session as unlocked even though it had been locked; Microsoft declined to change it. (https://securityaffairs.com/86584/hacking/cve-2019-9510-windows-flaw.html) Any redirect-to-RDP design inherits the question "what is the console's state afterwards?"
- [S] Sunshine issue #964 reports that a script which reconnects a session to the console leaves the system unlocked. (https://github.com/LizardByte/Sunshine/issues/964)
- [S-weak] Microsoft Q&A reports lock-screen and UAC hangs on 24H2 domain-joined machines with no stated root cause. (https://learn.microsoft.com/en-us/answers/a/1983959)

### 9. Virtual displays

- [C] An Indirect Display Driver (IDD) is a UMDF driver. "The IDD runs in Session 0 without any components running in the user session." Documented use cases include remote display and virtual monitors. (https://learn.microsoft.com/en-gb/windows-hardware/drivers/display/indirect-display-driver-model-overview)
- [C] IddCx 1.10 (Windows 11 23H2) adds HDR10/WCG; 1.11 adds D3D12. (https://learn.microsoft.com/windows-hardware/drivers/display/iddcx-versions)
- [C] The adapter flag `IDDCX_ADAPTER_FLAGS_REMOTE_SESSION_DRIVER` (IddCx 1.4) means a remote-session driver cannot support console-session monitors, and the reverse. (https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/iddcx/ne-iddcx-iddcx_adapter_flags)
- Negative result: I found no user-mode API to create a virtual monitor on Windows 11; the Windows 11 display-driver "what's new" page is silent (https://learn.microsoft.com/en-us/windows-hardware/drivers/display/what-s-new-for-windows-11-display-and-graphics-drivers), and so was a Build 2026 recap on blogs.windows.com (URL not recorded). [I] A driver is required.
- [C] Signing: attestation signing needs an EV certificate, is described as "for testing purposes only", cannot be used for Windows Update; Windows Server 2016 and later accept only HLK-passed, dashboard-signed drivers. (https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings ; https://learn.microsoft.com/pl-pl/windows-hardware/drivers/dashboard/code-signing-reqs) [I] A project that wants its own driver needs a Hardware Dashboard account, an EV certificate and an HLK run; the realistic alternative is an optional download of an existing signed MIT driver.
- [S] Existing drivers: Virtual Display Driver (MIT, IddCx 1.10, release binaries signed through SignPath Foundation) (https://github.com/VirtualDrivers/Virtual-Display-Driver); SudoVDA (MIT/CC0, used by Apollo, HDR on Windows 11 24H2) (https://github.com/SudoMaker/SudoVDA); Parsec's virtual display (https://support.parsec.app/hc/en-us/articles/32381178803604); RealVNC (https://help.realvnc.com/hc/en-us/articles/31739688741405-Adding-Virtual-Displays) and Splashtop (https://support-splashtopbusiness.splashtop.com/hc/en-us/articles/16913613811355-Windows-Virtual-display) ship virtual displays too; Splashtop says signing out removes them. RealVNC and Splashtop pages say nothing about the lock. Moonlight is a client; Sunshine and Apollo are the hosts.
- Unknown: whether a window on an IDD monitor stays composed and capturable while the session is locked, and whether the monitor survives lock, sign-out, sleep and fast user switching. No source. Because the IDD runs in session 0 [C], a virtual monitor is the one place where a lock might not interrupt frames, but that is a hypothesis only. Test T16.
- Unknown: whether the SignPath-signed VDD loads on a stock Windows 11 machine with Secure Boot on and no test signing. Test T16.

### 10. Detecting lock, unlock, sign-out, sleep, resume (Rust, no visible window)

Full API list and design in section (d). Summary:
- [C] Session events: `WTSRegisterSessionNotification` then `WM_WTSSESSION_CHANGE`; services use `SERVICE_CONTROL_SESSIONCHANGE`.
- [C] State polling: `WTSQuerySessionInformation(WTSSessionInfoEx)` `SessionFlags`.
- [C] Sleep/resume: `RegisterSuspendResumeNotification` (window) or `PowerRegisterSuspendResumeNotification` (callback, no window); display and presence through power-setting GUIDs.
- [C] SENS is not usable: `ISensLogon::DisplayLock`/`DisplayUnlock` "will not work with multiple logins through Remote Desktop Services and does not support Remote Desktop Services or Fast-User Switching". (https://learn.microsoft.com/en-us/windows/win32/api/sensevts/nf-sensevts-isenslogon-displaylock)
- [S] Chromium observes sessions through a hidden top-level window (`SingletonHwnd`); Electron emits `lock-screen` / `unlock-screen` from `WTS_SESSION_LOCK` / `UNLOCK`; .NET `SystemEvents.SessionSwitch` uses a hidden window on its own thread (sources in section (d), D.2).
- Unknown: whether a message-only window receives `WM_WTSSESSION_CHANGE` (docs say message-only windows "do not receive broadcast messages"); use a hidden top-level window (T09).

### 11. Auto-start and process model

11.1 Auto-start options [C unless noted]
- Run/RunOnce keys: at logon, may be delayed, ignored in Safe Mode by default (Q2 URL). `HKCU` Run is per-user and needs no elevation; `HKLM` Run covers all users [I].
- Startup folder: logon-time, same as Run [I, not separately sourced].
- Task Scheduler LogonTrigger: at logon (Q2 URL); BootTrigger is for before-logon work and needs an Administrator.
- Services: boot-start, session 0, session-change callbacks (Q2, Q10).

11.2 Process model I would propose [I]
1. Default (per-user installer, no elevation): one per-user agent started at sign-in through `HKCU\...\Run`. It listens on the network, owns the clipboard and file serving, detects lock/sleep, and does `LockWorkStation()` after a hand-off. It works for "locked and awake".
2. Opt-in "advanced" (per-machine installer, UAC): a LocalSystem service with `SERVICE_ACCEPT_SESSIONCHANGE`, started automatically. It (a) relaunches helpers on `WTS_CONSOLE_CONNECT`, (b) optionally runs a minimal SYSTEM helper on the console session for lock-screen capture and input, (c) optionally listens before sign-in. The network-facing parts stay in the unprivileged agent; the helper exposes only a tiny local IPC. Sunshine runs its whole application as SYSTEM; that is not recommended for Baton because the network-facing part would then be SYSTEM.
3. Optional separate download: a virtual-display driver (Q9).

11.3 Fast user switching [C][S][I]
- [C] SENS does not support fast user switching or RDS (Q10), so use WTS events: `WTS_CONSOLE_CONNECT`/`WTS_CONSOLE_DISCONNECT` and `WTS_REMOTE_CONNECT`/`WTS_REMOTE_DISCONNECT`.
- [S] Sunshine's service restarts its helper on `WTS_CONSOLE_CONNECT`; RustDesk polls the active console session and relaunches. (sources in Q7)
- [I] Per-user agents in several sessions run at the same time. They must not bind the same port, and only the agent in the active console session should accept UI-affecting work. DAM behaviour for a disconnected (switched-away) session is not documented; test T18.

11.4 Antivirus, SmartScreen, signing (as of the Microsoft pages updated 2026-08-29)
- [C] Azure Artifact Signing (formerly Trusted Signing): organisations in the USA, Canada, EU and UK; individuals in the USA and Canada only; about $9.99 per month; no instant SmartScreen trust. EV certificates have not bypassed SmartScreen since 2024; OV certificates need an HSM since June 2023; unsigned or self-signed files get a strong block; unsigned files start with zero reputation per version. (https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options ; https://learn.microsoft.com/et-ee/windows/apps/package-and-deploy/smartscreen-reputation)
- [C] Smart App Control in enforcement mode blocks unsigned code that lacks reputation. (https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/overview)
- [S] The Virtual Display Driver project says its releases are signed through the SignPath Foundation, which provides free code signing to open-source projects (Q9); the Foundation's own terms were not read.
- [I] Treat a signed installer plus a signed service and helper as mandatory for Baton. The SYSTEM helper will attract more AV scrutiny than the agent (T22).

### 12. Dynamic Lock, Windows Hello, Cross-device resume, Phone Link

- [C] Dynamic Lock locks the PC when a Bluetooth-paired phone's signal falls and the system is idle; no timing is stated; GPO "Configure dynamic lock factors". It only locks. (https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/hello-feature-dynamic-lock ; I read the sv-se mirror of this page)
- [C] Presence sensing (needs hardware) locks the screen on leave and wakes on approach; the page does not clearly distinguish screen-off from lock and does not mention sleep. (https://www.microsoft.com/windows/tips/presence-sensing)
- [C] The Cross-device resume page (Windows 11, Android 10+, supported apps Spotify, vivo Browser and WhatsApp as of Jan 2026), the Phone Link requirements, FAQ and calls pages are all silent on behaviour when the PC is locked or asleep. (https://support.microsoft.com/en-us/windows/apps/phonelink/cross-device-resume-feature ; https://support.microsoft.com/phone-link ; https://support.microsoft.com/help/17166 ; https://support.microsoft.com/en-us/windows/apps/phonelink/frequently-asked-questions-about-the-phone-link ; https://support.microsoft.com/en-us/windows/apps/make-and-receive-phone-calls-from-your-pc)
- [S-weak] "Lock PC" from Link to Windows on Android works over the internet and needs Remote PC controls; it locks, there is no unlock; behaviour when the PC is asleep or offline is not stated. (https://betanews.com/article/microsoft-adds-pc-locking-option-to-link-to-windows-app-for-android/ ; https://pureinfotech.com/windows-11-remote-lock-android/)
- [I] Microsoft itself offers remote lock but no remote unlock. That supports Baton's "lock on hand-off, never unlock remotely" stance, and a Baton "lock source device" action is consistent with platform norms. Windows Hello behaviour at the lock screen was not researched beyond the Dynamic Lock page. Tests T24, T25.

---

## (c) Must verify on real hardware

Each item says what uncertainty it settles, then the exact test. Priority: P0 decides the architecture, P1 decides features, P2 is polish. Record for every run: Windows edition and build (`winver`), power model, AC or DC, NIC type (wired/Wi-Fi), BitLocker mode, and whether the display was on or off.

### Test tools used by every test

- **Probe**: a small Rust program (the same detection code Baton will need) with two modes: `user` (started from `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`) and `service` (LocalSystem, auto start). A third mode, a SYSTEM helper in the console session, is not part of this copy. Each mode appends one line per second to `C:\ProgramData\baton-probe\<mode>.log` with: timestamp in ms, pid, own session id, `WTSGetActiveConsoleSessionId()`, `WTSQuerySessionInformation(WTSSessionInfoEx).SessionFlags`, result and desktop name of `OpenInputDesktop` + `GetUserObjectInformation(UOI_NAME)`. It also listens on TCP 47001 and answers `pong <timestamp>`. It logs, as they occur: `WM_WTSSESSION_CHANGE` from a hidden top-level window and from a message-only window (separately labelled); `WM_POWERBROADCAST` from `RegisterSuspendResumeNotification`; the callback from `PowerRegisterSuspendResumeNotification`; changes of `GUID_CONSOLE_DISPLAY_STATE`, `GUID_SESSION_DISPLAY_STATUS`, `GUID_SESSION_USER_PRESENCE`; `SERVICE_CONTROL_SESSIONCHANGE` in service mode.
- **Pinger** on a second LAN machine (PowerShell), one TCP probe per second, logging OPEN or TIMEOUT:

```powershell
$ip = "192.168.1.50"   # device under test
while ($true) {
  $t = Get-Date -Format o
  $c = New-Object Net.Sockets.TcpClient
  $r = $c.BeginConnect($ip, 47001, $null, $null)
  if ($r.AsyncWaitHandle.WaitOne(800) -and $c.Connected) { "$t OPEN" } else { "$t TIMEOUT" }
  $c.Close(); Start-Sleep 1
}
```

- **Lock** from a script: `rundll32.exe user32.dll,LockWorkStation`. **Display timeout**: `powercfg /change monitor-timeout-ac 1` (and `-dc`). **Never sleep**: `powercfg /change standby-timeout-ac 0` (and `-dc`).
- **Wake-on-LAN** from the second machine:

```powershell
$mac = "AA-BB-CC-DD-EE-FF" -split '-' | ForEach-Object { [byte]("0x$_") }
$pkt = [byte[]](,0xFF * 6) + ($mac * 16)
$u = New-Object Net.Sockets.UdpClient; $u.Connect([Net.IPAddress]::Broadcast, 9); [void]$u.Send($pkt, $pkt.Length)
```

- **Hardware and OS matrix** (as many as available): Windows 11 Pro 25H2 or 26H2 desktop on wired LAN; a Modern Standby laptop on Wi-Fi (Pro) run on AC and on battery; one S3-capable machine (`powercfg /a` shows "Standby (S3)", BIOS may need the setting); a Windows 11 Home machine; a 24H2 machine (service suspension changed in 24H2); an Arm64 machine on 26H1 (Snapdragon X2, different Windows core) if obtainable. TPM-only and TPM+PIN BitLocker configurations (`manage-bde -status`, `manage-bde -protectors -add C: -TPMAndPIN`), possibly in VMs for the boot tests.

### P0 tests

**T01 Power model and standby networking.** On each machine run `powercfg /a`, `powercfg /requests`, `powercfg /devicequery wake_armed`, `powercfg /qh > qh.txt`, and PowerShell `Get-NetAdapterPowerManagement`. Record whether it says "Standby (S0 Low Power Idle) Network Connected" or "Standby (S3)", which NICs are wake-armed, and any standby-connectivity setting in `qh.txt`. Drives: which column of table (a) applies per machine.

**T02 Per-user agent liveness while locked.** Probe `user` autostarts; Pinger runs. `powercfg /change monitor-timeout-ac 1`, `standby-timeout-ac 0`. Sign in, lock with Win+L, leave 15 minutes with the screen going off. Pass: 1 Hz ticks for the whole period and no TIMEOUT. Repeat on DC power. Then set `standby-timeout-ac 5` and see whether ticks stop before the machine visibly sleeps (answers: does display-off alone engage DAM on Modern Standby?). Drives: row 1 and note 2 of table (a).

**T03 Clipboard while locked.** Probe `user` exposes a command on the TCP port: `setclip <text>`. It calls `OpenClipboard`, `EmptyClipboard`, `SetClipboardData(CF_UNICODETEXT)`, `CloseClipboard`, logs every return value and `GetLastError`, and reads back with `GetClipboardData` plus `GetClipboardSequenceNumber`. It also registers `AddClipboardFormatListener` and logs each `WM_CLIPBOARDUPDATE`. Lock, send the command from the second machine, wait, unlock, paste into Notepad. Then change the clipboard from the unlocked state before locking and check the listener fires for a second process's change while locked (a scheduled task "run only when user is logged on" can do that). Repeat with the display off and 5 minutes after lock. Pass: set succeeds, read-back matches, paste shows the text.

**T04 Opening a link and a file while locked.** Probe command `open <url>` calls `ShellExecuteW("open", url)` on a worker thread with a 5 s watchdog, logs return value and duration. Cases: Edge, Chrome and Firefox each already running and not running; URL and a local PDF/TXT file. After unlock, check whether the tab or document is open and focused, and whether any launch hung. Pass: returns within the watchdog and the content is present on unlock.

**T05 Window capture matrix while locked.** Write a capture harness with an on-screen animated test window (a counter updated every 100 ms, plus the wall clock) and, once per second, try: (1) `DuplicateOutput` + `AcquireNextFrame`; (2) Windows.Graphics.Capture on the window and on the monitor, hashing each frame to see whether pixels change; (3) `PrintWindow(PW_RENDERFULLCONTENT)`; (4) `BitBlt` from the window DC; (5) the same four as SYSTEM using `psexec -s -i <session>` (Sysinternals, test use only) with `OpenInputDesktop` + `SetThreadDesktop`. Run unlocked for 30 s, lock for 60 s, unlock. Record per API: HRESULT or error, whether frames arrive, whether the counter advances, whether the frame shows the lock screen. Repeat with the display off. Pass criterion for "window streaming while locked is possible": at least one standard-user API delivers a live frame of the animated window during the lock. Drives: whether streaming from a locked device is possible at all (the biggest design fork).

**T06 Modern Standby liveness and wake.** On a Modern Standby laptop, Probe `user` and Probe `service` run, Pinger runs. Enter sleep three ways (Start menu Sleep, lid close with the lid action set to Sleep, idle timeout), on AC and on battery. Record: when the Pinger starts timing out (DAM delay), whether `PBT_APMSUSPEND` was logged first, whether the service keeps ticking (throttled 1 s per 30 s? fully suspended on 24H2+?), `powercfg /sleepstudy /output C:\sleep.html`. Then repeat while Probe holds `PowerCreateRequest` + `PowerSetRequest(PowerRequestExecutionRequired)` and then `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`; check `powercfg /requests` shows them; measure for 10 minutes on AC and 10 minutes on DC. Then send a Wake-on-LAN magic packet during sleep and record whether the machine wakes, how long it stays awake, whether the display turns on, and whether Probe `user` ticks while the display stays off (does a network wake un-suspend desktop apps?). Drives: columns "Asleep" in table (a).

**T07 S3 liveness and wake.** On S3 hardware, same as T06 minus the DAM parts. Send WoL (magic packet, then pattern/TCP if the NIC supports it: `Get-NetAdapterPowerManagement` shows `WakeOnMagicPacket` and `WakeOnPattern`). Record: wake success, time until Probe answers, how long the machine stays awake with nobody present (the unattended sleep timeout: `powercfg /qh SCHEME_CURRENT SUB_SLEEP`), whether the session comes back locked. Drives: a "wake, deliver, re-sleep" design for desktops.

**T08 Before sign-in: service reachability under BitLocker modes.** Install Probe `service` (`sc.exe create BatonProbe binPath= "C:\probe\probe.exe service" start= auto obj= LocalSystem`). For each of: no BitLocker, TPM-only, TPM+PIN; Home and Pro: restart to the sign-in screen without signing in; Pinger from the second machine; also `shutdown /s /t 0` then power on (Fast Startup) and `shutdown /s /hybrid /t 0`. Record: when the service starts relative to network up, whether inbound TCP works at the sign-in screen (also check the Windows Firewall profile applied at that point: `Get-NetConnectionProfile` after sign-in, and test with the inbound rule scoped to Private versus Any), whether anything runs before the PIN. Drives: Q2 verdict.

**T09 Lock-state detection correctness.** From Probe logs, over 20 cycles each of: Win+L, Ctrl+Alt+Del then Lock, `LockWorkStation()` from Probe `user`, idle timeout lock, Dynamic Lock, switch user, RDP connect and disconnect, sleep and resume. Compare for each cycle: time of `WTS_SESSION_LOCK`/`UNLOCK` in the hidden top-level window versus message-only window versus service callback; `SessionFlags` value and polling lag; `OpenInputDesktop` result (success or ACCESS_DENIED) and desktop name; Probe started while already locked. Pass: top-level window gets every lock and unlock within 1 s; record whether the message-only window gets any; record whether `SessionFlags` and the `OpenInputDesktop` heuristic agree with the events.

### P1 tests

**T10 Toasts on the lock screen.** From a signed test app that is registered as a toast sender (Start-menu shortcut with AppUserModelID), send four toasts with `scenario` default, `reminder`, `alarm`, `incomingCall` while the PC is locked, for each combination of Settings > System > Notifications "Show notifications on the lock screen" and "Show reminders and incoming VoIP calls on the lock screen" (on/off), and with Do Not Disturb on/off. Record which appear on the lock screen, whether sound plays, whether the display wakes if it was off, what clicking does before sign-in, and what remains in the notification centre after unlock.

**T11 File reads while locked.** Probe command `read <path>` with timing. Files: a local file in Documents; a OneDrive file made cloud-only (right-click > Free up space) checking `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (0x400000); a cloud-only file under a Known-Folder-Moved Desktop; an EFS file (`cipher /e`). Run unlocked, locked 1 minute, locked 30 minutes, display off. Run the same list from Probe `service` at the sign-in screen (nobody signed in) and with a user signed in but locked. Record success, latency, error codes; whether OneDrive is running (`Get-Process OneDrive`).

**T12 SYSTEM helper at the lock screen (not reproduced).** The procedure is left out of this copy. Baton does not plan to build a SYSTEM helper (locked-devices.md, section 5.5), and no Baton rule depends on the answer.

**T13 Input injection while locked.** (1) Probe `user`: `SendInput` of a key and a mouse move while locked, with a Notepad window (check text after unlock) and a low-level hook logging the events. (2) A UI Automation `InvokePattern` call on a button of a test WinForms app while locked. Record what reaches the user's windows. The part of this test that used a SYSTEM helper at the lock screen is not reproduced in this copy, because Baton never types into a lock screen.

**T14 RDP behaviour on the console.** On Windows 11 Pro: (a) the same user, signed in at the console with Notepad open and Probe running, connects with `mstsc` from another PC; (b) a different user connects; (c) after each, record `qwinsta`, the console's visible state (lock screen, sign-in screen, logged off), whether Probe keeps its session id and keeps ticking; (d) disconnect by closing the window and by signing out the RDP session, and record whether the console is locked or unlocked (CVE-2019-9510 area); (e) from the RDP session run `tscon <rdp session id> /dest:console` and record the console state. On Windows 11 Home: confirm that port 3389 is not listening and the Settings toggle is absent.

**T15 Loopback RDP and CRD curtain.** While signed in at the console with Notepad open, run `mstsc /v:127.0.0.2` (RDP enabled, NLA off on the test machine) and sign in as the same user. Record whether the existing session (Notepad open) is attached to the RDP window or a new empty session is created, and what the physical console shows. Optional: install Chrome Remote Desktop with `RemoteAccessHostRequireCurtain=1` and the registry values from Q8.2 and connect from another device; record what the physical console shows and what happens when a key is pressed on it.

**T16 Virtual display while locked.** On a machine with Secure Boot on and no test signing, install the VDD or SudoVDA release. Record whether install succeeds without warnings. Create a virtual monitor, move the animated test window of T05 onto it, run the T05 harness against the monitor (`DuplicateOutput` on that output, WGC on the monitor and window) through lock, sign-out and sign-in, sleep and resume, and fast user switching. Record whether the monitor persists, frames continue, and the window survives.

**T17 Restart and update-restart start-up.** (a) Restart, sign in: time from desktop to the first Probe tick for HKCU Run, Startup folder, and a logon-trigger task. (b) Shut down, power on (Fast Startup): same. (c) Hibernate and resume: does Probe survive, does its network recover. (d) On a BitLocker machine, run an update restart or `shutdown /g /t 0` and observe whether the user is signed in automatically and locked (ARSO), whether HKCU Run entries start the agent, and whether Probe answers the Pinger while the screen shows the lock screen.

**T18 Fast user switching.** User A signed in with Probe `user`, lock A, sign in as B at the console (Probe for B listens on the same port). Record A's ticks, port conflicts, `WTSGetActiveConsoleSessionId()` and the WTS events on both agents (`WTS_CONSOLE_DISCONNECT`/`CONNECT`), and whether A's clipboard and file reads still work while A is disconnected.

**T19 Resume behaviour.** After each T06/T07 wake: is the session locked ("require sign-in" setting), the sequence of `PBT_*` and `WTS_*` events, `GUID_CONSOLE_DISPLAY_STATE` changes, how long until the NIC link is back (Wi-Fi), and whether an existing QUIC connection from the peer is dead (peer keep-alive/idle timeout) and how long a re-handshake takes.

**T20 Remote Desktop-enabled standby network.** On a Modern Standby laptop on battery, compare Pinger results with Remote Desktop off versus on (Settings > System > Remote Desktop) after 10 minutes of sleep; record battery use from the sleep study. Also try the Windows policy that controls network connectivity in standby if present (Appendix C) and record its effect.

**T21 Lock on hand-off.** From Probe `user`, call `LockWorkStation()` on command and record success and latency; call it from Probe `service` (session 0) and record the result (expected failure). Check idempotence when already locked and the event sequence.

**T22 Installer, SmartScreen, Smart App Control, Defender.** On a clean Windows 11 VM with Smart App Control in enforcement mode and Defender cloud protection on: download the installer in a browser (Mark of the Web), run it, and record the SmartScreen and SAC outcome for: unsigned; signed with Azure Artifact Signing or an OV certificate; signed by SignPath Foundation. Then install the service and SYSTEM helper and record any Defender detections or ASR prompts, including during `CreateProcessAsUser`, `SetThreadDesktop(Winlogon)` and `SendInput`.

### P2 tests

**T23 RDP shadow of a locked session.** On Windows 11 Pro or Windows Server: `mstsc /v:<host> /shadow:<id> /control /noConsentPrompt` against a session that is then locked; then trigger a UAC prompt. Record the shadow window content (expected: black with a pause symbol).

**T24 Dynamic Lock and presence sensing.** Pair a phone, enable Dynamic Lock, walk away; record time from leaving to lock, whether the machine is then asleep or awake with the screen off, and what Probe logged. Same for a presence-sensing machine.

**T25 Phone Link and Cross-device resume.** With a paired Android phone: lock the PC and send a text and a notification; start a supported app (Spotify) and look for the resume prompt; repeat with the PC asleep on Modern Standby; use "Lock PC" from Link to Windows with the PC asleep and offline. Record what appears on the lock screen and whether the PC wakes.

**T26 OS and hardware matrix repeat.** Repeat T02, T05 and T06 on 24H2, 25H2 and 26H2 x64 and, if available, on 26H1 Arm64 (Snapdragon X2). Confirm that nothing changes across builds, and that an Arm64 build of Probe and the IDD driver are available.

**T27 Rust compile check.** In a scratch crate, `cargo add windows` and compile each API in section (d) to record the actual module paths and required feature names (crates.io shows 0.62.2 as latest; GitHub master shows 0.100.0 with different naming), and whether `windows-service` exposes a session-change variant of `ServiceControl`.

**T28 `SendSAS` path (not reproduced).** Left out of this copy. Baton has no use for it.

---

## (d) API names for lock and sleep detection

Legend for this section: [C] the function and constants were read on the cited Microsoft Learn page; [S] seen in shipping open-source code; [I] standard usage I did not re-read here.

### D.1 The table

| Need | In the user-session agent (Rust, hidden window) | In a Windows service | Notes |
|---|---|---|---|
| Lock / unlock | `WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)` then `WM_WTSSESSION_CHANGE` with `wParam` = `WTS_SESSION_LOCK` (0x7) or `WTS_SESSION_UNLOCK` (0x8); `lParam` = session id; undo with `WTSUnRegisterSessionNotification` [C] | `RegisterServiceCtrlHandlerExW`, accept `SERVICE_ACCEPT_SESSIONCHANGE` (flag seen in Sunshine's service [S]), receive `SERVICE_CONTROL_SESSIONCHANGE` with `dwEventType` = the same `WTS_*` values and `lpEventData` = `WTSSESSION_NOTIFICATION` [C] | `LockWorkStation` doc recommends this route [C] |
| Sign-in / sign-out | `WTS_SESSION_LOGON` (0x5), `WTS_SESSION_LOGOFF` (0x6) [C] | same [C] | a desktop app never sees its own logon [S-weak: .NET analysis]; a service sees a logon only if it was fully loaded before it [C] |
| Console attach / detach, RDP | `WTS_CONSOLE_CONNECT` (0x1), `WTS_CONSOLE_DISCONNECT` (0x2), `WTS_REMOTE_CONNECT` (0x3), `WTS_REMOTE_DISCONNECT` (0x4), `WTS_SESSION_REMOTE_CONTROL` (0x9) [C] | same [C] | covers fast user switching and RDP, which SENS does not [C]; RDP reconnect appears as ConsoleDisconnect then RemoteConnect [S-weak] |
| Current lock state (poll) | `WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSSessionInfoEx, ...)` returns `WTSINFOEXW`; read `Data.WTSInfoExLevel1.SessionFlags`: `WTS_SESSIONSTATE_LOCK` = 0, `WTS_SESSIONSTATE_UNLOCK` = 1, unknown = 0xFFFFFFFF; free with `WTSFreeMemory` [C] | same with an explicit session id [C] | values are reversed on Windows 7 / Server 2008 R2, irrelevant now; Chromium caches the result because the call is expensive (crbug 940607) [S] |
| Suspend / resume (needs a window) | `RegisterSuspendResumeNotification(hwnd, DEVICE_NOTIFY_WINDOW_HANDLE)` then `WM_POWERBROADCAST` with `PBT_APMSUSPEND`, `PBT_APMRESUMESUSPEND`, `PBT_APMRESUMEAUTOMATIC` [C] | `PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK, &DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, &HPOWERNOTIFY)` [C] | Chromium: "On machines with modern standby calling RegisterSuspendResumeNotification is required in order to get the PBT_APMSUSPEND message"; it treats `PBT_APMRESUMEAUTOMATIC` as resume and ignores `PBT_APMRESUMESUSPEND` [S] |
| Suspend / resume (no window) | `PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK, ...)` (Win8+, `powerbase.h`, `Powrprof.lib`) with callback `DEVICE_NOTIFY_CALLBACK_ROUTINE`; undo with `PowerUnregisterSuspendResumeNotification` [C] | same [C] | use this if no window thread is wanted; the callback runs on a system thread, so keep it short |
| Display on/off, user presence | `RegisterPowerSettingNotification(hwnd, &GUID, DEVICE_NOTIFY_WINDOW_HANDLE)` then `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE` [C for the function, I for the message name]; GUIDs below | `PowerSettingRegisterNotification(&GUID, DEVICE_NOTIFY_CALLBACK, ...)` (Win7+, callback is invoked at once with the current value) or `RegisterPowerSettingNotification(..., DEVICE_NOTIFY_SERVICE_HANDLE)` [C] | GUIDs: `GUID_CONSOLE_DISPLAY_STATE` 6FE69556-704A-47A0-8F24-C28D936FDA47 (Win8+); `GUID_SESSION_DISPLAY_STATUS` 2B84C20E-AD23-4DDF-93DB-05FFBD7EFCA5 (user-mode only, not session 0); `GUID_SESSION_USER_PRESENCE` 3C0F4548-C03F-4C4D-B9F2-237EDE686376 (session 0 should use `GUID_GLOBAL_USER_PRESENCE`) [C] |
| Power model | `CallNtPowerInformation(SystemPowerCapabilities, ...)` then `SYSTEM_POWER_CAPABILITIES.AoAc` is TRUE for Modern Standby [C: the DAM page names this test] | same | runtime detection of Modern Standby vs S3 |
| Start a lock | `LockWorkStation()` from a process on the interactive desktop [C] | not from session 0 [I] | asynchronous; confirm with `WTS_SESSION_LOCK` |
| Keep awake | `SetThreadExecutionState` with `ES_CONTINUOUS` combined with `ES_SYSTEM_REQUIRED`; `PowerCreateRequest`, `PowerSetRequest(PowerRequestExecutionRequired)`, `PowerClearRequest` [C] | same | limits in Q1.5 |
| Console session id | `WTSGetActiveConsoleSessionId()` [C]; `ProcessIdToSessionId` for the agent's own session [I] | same | CRD compares the two to decide "curtained" [S] |
| Launch helper in a session | not applicable | `WTSQueryUserToken` (LocalSystem + SE_TCB) [C], or duplicate the service token and `SetTokenInformation(TokenSessionId)` [S Sunshine], then `CreateProcessAsUserW` with `lpDesktop` = `winsta0\default` [S] | attaching to the input desktop: `OpenInputDesktop(DF_ALLOWOTHERACCOUNTHOOK, FALSE, GENERIC_ALL)` then `SetThreadDesktop` [S Sunshine]; CRD uses `SwitchToInputDesktop` [S] |

### D.2 Recommended Rust design [I]

1. Dedicated thread "session-events": register a window class, create a hidden top-level `WS_POPUP` window of size 0 with `WS_EX_NOACTIVATE` (Chromium's power monitor does exactly this [S]; its session observer uses a hidden top-level `SingletonHwnd` and filters by the current session [S]; Electron emits `lock-screen` / `unlock-screen` from `WTS_SESSION_LOCK` / `UNLOCK` [S]; .NET `SystemEvents.SessionSwitch` creates a hidden window on its own thread [S-weak]). Do not use a message-only window (HWND_MESSAGE): the docs say message-only windows "do not receive broadcast messages" and say nothing about `WM_WTSSESSION_CHANGE` (https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features ; https://devblogs.microsoft.com/oldnewthing/20171218-00/?p=97595). Unknown, test T09.
2. On that window call, in order: `WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)`, `RegisterSuspendResumeNotification(hwnd, DEVICE_NOTIFY_WINDOW_HANDLE)`, and `RegisterPowerSettingNotification` for `GUID_CONSOLE_DISPLAY_STATE` and `GUID_SESSION_USER_PRESENCE`. Run a `GetMessageW` loop. On shutdown unregister before `DestroyWindow` (documented requirement [C]).
3. If `WTSRegisterSessionNotification` fails with `RPC_S_INVALID_BINDING`, TermService is not ready: wait on the event `Global\TermSrvReadyEvent` and retry [C].
4. At start-up, and again after every `PBT_APMRESUMEAUTOMATIC`, poll `WTSQuerySessionInformation(WTSSessionInfoEx)` to learn the true state (the agent may start inside an already-locked session after ARSO, Q2.3). Treat the events as the source of truth afterwards and the poll as a correction.
5. Treat every resume as "all peer connections are dead": drop QUIC connections, re-announce on the LAN, reconnect to the relay (T19).
6. On `PBT_APMSUSPEND` flush state and close connections: under Modern Standby the process is suspended "after a few seconds delay" [C] and receives nothing afterwards.
7. Optional secondary signals: `OpenInputDesktop` failing with access denied while the Winlogon desktop is active [I, T09]; RustDesk's `is_logon_ui` (LogonUI.exe in this session) and `is_locked` flags [S], noting "a LogonUI.exe can outlive the unlock".
8. A service, if present, uses `SERVICE_CONTROL_SESSIONCHANGE` and forwards `lock`/`unlock`/`console-connect` to the user agents over a named pipe. The `windows-service` crate (0.8.1) exposes `ServiceControlAccept::SESSION_CHANGE` [C: https://docs.rs/windows-service/latest/windows_service/service/struct.ServiceControlAccept.html]; whether its `ServiceControl` enum has a session-change variant I could not verify (Unknown; fall back to `RegisterServiceCtrlHandlerExW` through the `windows` crate).

### D.3 Rust crate notes

- [S] crates.io sparse index lists `windows` 0.62.2 as the latest published version, with features including `Win32_System_RemoteDesktop`, `Win32_System_Power`, `Win32_UI_WindowsAndMessaging`, `Win32_System_StationsAndDesktops`, `Win32_System_Services`, `Win32_Foundation`, `Win32_System_Shutdown` (https://index.crates.io/wi/nd/windows). GitHub master shows version 0.100.0 with a different, header-based feature naming; I could not confirm it is published, so do not rely on it (https://github.com/microsoft/windows-rs).
- [I] Likely module placement (docs.rs function pages could not be fetched): `WTS*` functions and `WTSINFOEXW` in `Win32::System::RemoteDesktop`; `Power*`, `HPOWERNOTIFY`, `DEVICE_NOTIFY_*`, `RegisterPowerSettingNotification`, `RegisterSuspendResumeNotification` and the `GUID_*` power settings in `Win32::System::Power`; window functions, `WM_WTSSESSION_CHANGE`, `WM_POWERBROADCAST`, `PBT_*` in `Win32::UI::WindowsAndMessaging`; `OpenInputDesktop`, `SetThreadDesktop` in `Win32::System::StationsAndDesktops`; service functions in `Win32::System::Services`; `LockWorkStation` in `Win32::System::Shutdown`. Confirm by compiling (T27).

---

## Appendix A. Windows versions in scope (Microsoft release-health page dated 2026-09-29)

Source: https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information [C]; 26H1 details: https://learn.microsoft.com/en-us/windows/whats-new/windows-11-version-26h1 [C].

| Version | General availability | End of updates Home/Pro | End of updates Enterprise/Education | Latest build on the page |
|---|---|---|---|---|
| 26H2 | 2026-09-29 | 2028-10-10 | 2029-10-09 | 26300.9550 |
| 26H1 | 2026-02-10 | 2028-03-14 | 2029-03-13 | 28000.3086 |
| 25H2 | 2025-09-30 | 2027-10-12 | 2028-10-10 | 26200.9550 |
| 24H2 | 2024-10-01 | 2026-10-13 (ended yesterday) | 2027-10-12 | 26100.9550 |
| 23H2 | 2023-10-31 | ended | 2026-11-10 | 22631.7584 |
| 24H2 LTSC | 2024-10-01 | - | 2029-10-09 (IoT Enterprise LTSC only: extended to 2034-10-10) | 26100.9550 |

- [C] 26H1 is scoped to new devices that reached the market in early 2026 (Snapdragon X2), is not offered as a Windows Update or in-place update, is based on a different Windows core than 24H2, 25H2 and the second-half 2026 release, and its devices cannot update to that release (a path is promised in a future release). The pages do not mention changes to lock screen, sessions, Remote Desktop, virtual display or clipboard behaviour.
- [S-weak] Windows 10 end of support was 2025-10-14; consumer ESU is said to end 2026-10-13, per Microsoft Q&A answers that are AI-generated or from independent advisors and not official (https://learn.microsoft.com/en-us/answers/a/12637085). I did not read the official lifecycle page.
- [I] Practical support floor: Windows 11 24H2 and later (DAM service suspension changed in 24H2 [C]); test on 25H2, 26H2 and the Arm64 26H1 core. The Win32 APIs in section (d) are the same on Windows 10 22H2, which is out of mainstream support.

## Appendix B. Conflicts, gaps and limits stated plainly

Conflicts
1. OBS forum posts disagree on whether recording works at the lock screen (2016 "the OS stops rendering the UI entirely when the screen is locked" versus 2022 "on Win 10, obs manages to record the lock screen"). Both [S-weak]. Not resolved. (T05, T12)
2. Looking Glass wiki says that even with an elevated helper service "only the Login Screen cannot be captured", while Sunshine and RustDesk code show SYSTEM processes attaching to the input desktop to capture the secure desktop. The Looking Glass capture path (GPU pass-through VM) differs from desktop duplication on a normal PC, but I cannot show that is the reason. (T12)
3. RDP versus the local user: community Q&A says the local user is logged off "by design"; WinAppDriver says closing Remote Desktop locks the computer; CRD curtain mode relies on the session continuing. No official Microsoft statement found. (T14, T15)
4. Windows Home cannot host RDP: stated on support.microsoft.com ("needs to be running a Pro edition") and in one fetch of a Learn page; the Q&A that repeats it is an AI-generated answer. Consistent, but only one official page is clear.
5. The Microsoft DAM page dates from Windows 8 wording ("connected standby") while the Modern Standby pages mention 24H2; page wording is mixed between Connected Standby and Modern Standby. Treated as describing the same mechanism.

Gaps (Unknown)
- Windows.Graphics.Capture, DirectComposition, `BitBlt`, `PrintWindow` output for user windows while locked (T05).
- Clipboard set and listener behaviour while locked (T03).
- `ShellExecute` while locked, and unpackaged-app toasts on the lock screen (T04, T10).
- OneDrive hydration and EFS while locked (T11).
- Whether DAM is engaged when the screen is off but the system has not slept, and whether a network wake with the display off un-suspends desktop apps (T02, T06).
- Whether message-only windows receive `WM_WTSSESSION_CHANGE` (T09).
- Whether CRD curtain mode attaches the existing session or creates a new one; how its loopback RDP authenticates (T15).
- AnyDesk and Parsec privacy-mode mechanisms.
- Sunshine's Windows keyboard and mouse injection path, and its lock-screen input behaviour.
- Virtual display behaviour while locked and Secure Boot loading of the signed community drivers (T16).
- Whether sideloaded MSIX background tasks could run during Modern Standby (not researched).
- Sources named in the brief that I did not read: OBS and ShareX capture source code (only OBS forum threads were read), FreeRDP, the Microsoft IddCx sample driver, and any Parsec privacy-mode documentation.
- Official Microsoft statements on consumer ESU dates, Windows 11 25H2/26H2 session-behaviour changes, and Windows Hello at the lock screen.

Limits
- The shared web-search budget was exhausted near the end. The three gap-filling searches (WGC while locked, clipboard while locked, same-user RDP versus console) returned "Web search was not performed" and I did not work around the limit. A new run with a higher budget could close some of the Unknown items by finding forum or source evidence, but T05 and T03 need hardware either way.
- docs.rs function pages could not be fetched (client errors), so the exact Rust module paths are inferred.

## Appendix C. Facts used from general knowledge and not verified in this research

- Group Policy "Allow network connectivity during connected-standby" (plugged in and on battery) and the `powercfg` standby-connectivity setting; policies that lock after inactivity ("Interactive logon: Machine inactivity limit").
- Task Scheduler: a SYSTEM-principal task runs in session 0; "run only when user is logged on" runs in that user's session.
- Toasts from unpackaged apps need a Start-menu shortcut that carries an AppUserModelID (and a COM activator) before they can be shown.
- DPAPI user-scope keys and EFS keys depend on the user's sign-in credentials.
- Windows consumer "device encryption" is TPM-only by default.
- The Settings label for the ARSO opt-out is under Accounts > Sign-in options ("Use my sign-in info to automatically finish setting up my device and reopen my apps after an update or restart").
- `PBT_APMRESUMEAUTOMATIC` fires on every resume and `PBT_APMRESUMESUSPEND` follows only when user input caused the resume; `PBT_POWERSETTINGCHANGE` is the sub-message of `WM_POWERBROADCAST` for power-setting notifications.
- `powercfg /a` wording, `powercfg /change`, `/requests`, `/sleepstudy`, `/devicequery wake_armed`, `/qh`; `Get-NetAdapterPowerManagement`; `manage-bde` switches; `shutdown /g`; `rundll32 user32.dll,LockWorkStation`.
