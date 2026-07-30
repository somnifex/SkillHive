# Accounts and sessions

## Register

Open **Register**, then provide a username, email address, and password. Usernames must be 3–50
characters from letters, numbers, `_`, `.`, and `-`. Passwords must be 8–128 characters and
include uppercase, lowercase, and a digit.

Registration creates a normal active user and a personal OpenAI-style starter template.

## Sign in and sign out

After sign-in, the short-lived access token remains in browser memory. A rotating refresh session
uses an HttpOnly cookie, so closing a tab does not necessarily end the session. Use **Sign out** to
revoke the current refresh session.

Repeated failed sign-ins can temporarily lock the combination of client address and account
identity. The default is five attempts followed by 15 minutes.

## Change your password

Open **Settings**, enter the current and new password, and submit. A successful password change
revokes existing refresh sessions, so other signed-in devices must authenticate again.

## Forgotten passwords

The current forgot-password endpoint deliberately returns the same accepted response for every
address, but no email is sent and no password is changed. Ask the installation's operator for
account recovery until a delivery workflow is implemented.

## Account suspension

A platform administrator can suspend an account. Suspended users cannot continue normal access.
Contact the operator if you believe your status is incorrect.
