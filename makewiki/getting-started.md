# Getting started

This walkthrough takes a local SQLite installation from first launch to a versioned private skill.

## 1. Install and start

Complete the [local installation](installation.md), then run:

```powershell
.\scripts\dev.ps1
```

Open <http://127.0.0.1:5173>. API documentation is at <http://127.0.0.1:8000/docs>.

## 2. Sign in

For an isolated local evaluation, use `admin` / `Admin123!`, or register a new account. Seed
credentials are public development data: never reuse them in a shared deployment.

Registration requires:

- a username of 3–50 letters, numbers, `_`, `.`, or `-`;
- a valid email address;
- a password of 8–128 characters containing uppercase, lowercase, and a digit.

Every new account receives a personal starter template using the recommended `SKILL.md` shape.

## 3. Create a skill from the starter template

1. Open **Templates**.
2. Find your personal OpenAI starter template.
3. Choose the action to create a skill from it.
4. Give the skill a unique slug and customize the generated content.
5. Save it; the result is private to your account.

See [Templates](usage/templates.md) for scope permissions and [Private skills](usage/private-skills.md)
for editing and version history.

## 4. Create a group

Open **Groups**, create a group, and select a joining policy. As its owner, you can invite members,
approve requests, create group templates, and enable published global skills. Invite-link joining
is not yet available even though it is reserved in the data model.

## 5. Stop the local services

Close the backend and frontend terminal processes. If you used Docker Compose:

```powershell
docker compose down
```

Your local SQLite data remains in `data/skillhive.db`.
