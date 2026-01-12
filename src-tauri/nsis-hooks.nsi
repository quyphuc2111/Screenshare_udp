; NSIS hooks for Screen Broadcast
; Adds Windows Firewall rules for UDP multicast

!macro CUSTOM_INSTALL_AFTER_INSTALL
  ; Add firewall rule for inbound UDP (Student receiving)
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Screen Broadcast UDP In" dir=in action=allow protocol=UDP localport=5000 profile=private,domain'
  
  ; Add firewall rule for outbound UDP (Teacher sending)
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Screen Broadcast UDP Out" dir=out action=allow protocol=UDP localport=5000 profile=private,domain'
  
  ; Allow the app through firewall
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Screen Broadcast App" dir=in action=allow program="$INSTDIR\screenshare_udp_native.exe" profile=private,domain'
!macroend

!macro CUSTOM_UNINSTALL_BEFORE_UNINSTALL
  ; Remove firewall rules on uninstall
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Screen Broadcast UDP In"'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Screen Broadcast UDP Out"'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Screen Broadcast App"'
!macroend
