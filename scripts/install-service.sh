#!/bin/sh
# Explicit, opt-in user service; never uses sudo or changes login policies.
set -eu
binary=${1:-"$HOME/.local/bin/teryon"}
test -x "$binary" || { echo "Pass an absolute installed teryon path" >&2; exit 1; }
case "$binary" in /*) ;; *) echo "Binary path must be absolute" >&2; exit 1;; esac
"$binary" stop local
case "$(uname -s)" in
  Darwin)
    directory="$HOME/Library/LaunchAgents"
    mkdir -p "$directory" "$HOME/.teryon/run"
    chmod 700 "$HOME/.teryon/run"
    file="$directory/dev.zyvor.teryon.plist"
    python3 - "$binary" "$file" "$HOME" <<'PY'
import os,plistlib,sys
binary,path,home=sys.argv[1:]
data={'Label':'dev.zyvor.teryon','ProgramArguments':[binary,'daemon'],'RunAtLoad':True,
      'StandardOutPath':home+'/.teryon/run/service.log','StandardErrorPath':home+'/.teryon/run/service.log'}
with open(path,'xb') as f: plistlib.dump(data,f)
os.chmod(path,0o600)
PY
    launchctl bootstrap "gui/$(id -u)" "$file"
    echo "Installed user LaunchAgent $file"
    ;;
  Linux)
    directory="$HOME/.config/systemd/user"
    mkdir -p "$directory"
    file="$directory/teryon.service"
    python3 - "$binary" "$file" <<'PY'
import os,sys
binary,path=sys.argv[1:]
# systemd ExecStart does not use shell quoting. Escape its own syntax instead.
escaped=binary.replace('\\','\\\\').replace('"','\\"').replace('%','%%')
with open(path,'x') as f:
 f.write('[Unit]\nDescription=TERYON persistent terminal workspace\n\n[Service]\nType=simple\nExecStart="'+escaped+'" daemon\n\n[Install]\nWantedBy=default.target\n')
os.chmod(path,0o600)
PY
    systemctl --user daemon-reload
    systemctl --user enable --now teryon.service
    echo "Installed user service $file"
    ;;
  *) echo "Only macOS and Linux user services are supported" >&2; exit 1;;
esac
