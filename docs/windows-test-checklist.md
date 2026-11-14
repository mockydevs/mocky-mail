# Windows test checklist (Store package)

Test on a real Windows 10 or 11 PC before submitting to the Microsoft Store.

## Install the test build
1. Turn on Developer Mode: **Settings > System > For developers > Developer Mode**.
2. Unzip `mocky-mail-store-test.zip` to a folder, for example the Desktop.
3. In that folder, right-click `install-test.ps1` > **Run with PowerShell**
   (or: `powershell -ExecutionPolicy Bypass -File .\install-test.ps1`).
4. Open **Mocky Mail** from the Start menu.

## Check
- [ ] The app opens, with the red envelope icon in Start and on the taskbar.
- [ ] Add a password account (for example info@mocky.co.ke). If it fails, write down the exact error text.
- [ ] Add a Gmail account with **Continue with Google**; the browser opens and returns to the app.
- [ ] Add an Outlook account with **Continue with Microsoft**.
- [ ] Mail loads, and opening a message shows its body.
- [ ] Send an email to yourself; it leaves the Outbox and arrives.
- [ ] With the app open, send yourself a new email: a Windows notification appears
      titled with the sender, under the name "Mocky Mail".
- [ ] Close and reopen the app: accounts and mail are still there, no password asked again.
- [ ] Links in an email open in the browser.
- [ ] Dark mode (sun/moon button) works.

## Remove the test build
`Get-AppxPackage *MockyMail* | Remove-AppxPackage` in PowerShell, then turn Developer Mode off.

## Report back
For anything that fails: what you did, the exact message, and a screenshot. If an
antivirus warns, note its name and the warning text.
