# Private skills

Private skills belong to one account. Searches, reads, edits, copies, and deletions are always
restricted to the owner, including when the caller is a platform administrator.

## Create a skill

Open **My Skills** and create a skill. Provide a readable name, a unique slug within your account,
an optional description, and the skill content. You can also create one from
[a template](templates.md).

Saving the skill creates its initial immutable version and makes that version current.

## Find and edit

Use search, filters, and pagination to find a skill. Editing its content creates a new version
rather than rewriting the previous version. Version history can therefore show how the current
content evolved.

Use meaningful version notes when the interface asks for them; they make rollback decisions and
reviews easier even though old version records themselves are immutable.

## Copy

Copying creates another private skill under your account. Use a distinct slug and then edit the
copy without affecting the source.

## Delete

Deletion is soft: the skill is removed from normal lists without erasing its underlying record.
The current UI does not provide a self-service restore action, so confirm before deleting.

## Use the generated Markdown

Skills created from a template contain generated `SKILL.md`-style Markdown and source-template
metadata. SkillHive does not currently provide a one-click downloadable skill package; copy the
content into your target skill directory when an external tool needs a file.
