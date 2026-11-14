# Running ufop-server with launchd

```sh
# set the server password first
/Applications/UFOP.app/Contents/MacOS/ufop-server --set-password
# per user, starts at login
cp com.ufop.server.plist ~/Library/LaunchAgents/
launchctl load -w ~/Library/LaunchAgents/com.ufop.server.plist
# or system-wide, starts at boot
sudo cp com.ufop.server.plist /Library/LaunchDaemons/
sudo launchctl load -w /Library/LaunchDaemons/com.ufop.server.plist
```

For the desktop app itself, enable **Start at login** in the Backups panel.
