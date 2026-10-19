#!/usr/bin/env bash
# Measuring only; not for main.
# Times how long an iPhone simulator takes to boot: the device that the iPhone job uses, and new
# ones of the same type, each in the order given.
#
#   diag-boot.sh preinstalled-first | new-first
set -u
order=${1:?usage: diag-boot.sh preinstalled-first | new-first}

udid=$(xcrun simctl list devices available | grep 'iPhone' | tail -n 1 | grep -oE '[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}')
info=$(xcrun simctl list devices available -j | python3 -c '
import json, sys
wanted = sys.argv[1]
for runtime, devices in json.load(sys.stdin)["devices"].items():
    for device in devices:
        if device["udid"] == wanted:
            print(runtime, device["deviceTypeIdentifier"])
' "$udid")
runtime=${info% *}
devtype=${info#* }
echo "device the iPhone job uses: $udid"
echo "runtime: $runtime"
echo "type: $devtype"

# boot ID LABEL: boots the device, says when it is listed as Booted and when the boot is over.
boot() {
  local id=$1 label=$2 start booted over waited=0
  start=$(date +%s)
  xcrun simctl boot "$id"
  until xcrun simctl list devices | grep "$id" | grep -q '(Booted)'; do
    sleep 1
    waited=$((waited + 1))
    if [ "$waited" -gt 900 ]; then echo "never listed as Booted"; break; fi
  done
  booted=$(date +%s)
  xcrun simctl bootstatus "$id"
  over=$(date +%s)
  echo "BOOT $label: listed as Booted after $((booted - start)) s, over after $((over - start)) s"
  xcrun simctl shutdown "$id"
}

# A new device of the type of the one the iPhone job uses; prints its id.
fresh() {
  xcrun simctl create "diag-$1" "$devtype" "$runtime"
}

case "$order" in
  preinstalled-first)
    boot "$udid" "preinstalled device, first boot"
    boot "$(fresh 1)" "new device, first boot"
    boot "$udid" "preinstalled device, second boot"
    boot "$(fresh 2)" "another new device, first boot"
    ;;
  new-first)
    boot "$(fresh 1)" "new device, first boot"
    boot "$udid" "preinstalled device, first boot"
    boot "$(fresh 2)" "another new device, first boot"
    boot "$udid" "preinstalled device, second boot"
    ;;
  *)
    echo "unknown order: $order" >&2
    exit 2
    ;;
esac
echo "done"
