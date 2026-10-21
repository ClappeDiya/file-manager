# Running ufop-server as a Windows service

`ufop-server.exe` registers itself with the Service Control Manager:

```powershell
# elevated PowerShell
ufop-server.exe --data-dir C:\ProgramData\ufop --set-password
ufop-server.exe --install-service --data-dir C:\ProgramData\ufop   # automatic start, LocalSystem
Start-Service ufop-server
ufop-server.exe --uninstall-service
```

The service runs `ufop-server.exe --windows-service <your arguments>`; any
other arguments given to `--install-service` are stored as the service's
start arguments. For start at login without a service, enable
**Start at login** in the desktop app's Backups panel.
