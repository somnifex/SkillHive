# Templates

Templates provide consistent starting instructions while keeping creation permissions separate
from use permissions.

## Scopes and permissions

| Scope    | Who can create or manage it? | Who can use it?       |
| -------- | ---------------------------- | --------------------- |
| Personal | Its owner                    | Its owner             |
| Group    | Group owner or administrator | Members of that group |
| Global   | Platform administrator       | Authenticated users   |

A platform administrator cannot use administration privileges to read another user's personal
template content.

## Default starter template

Each account receives one personal OpenAI-style starter template. Existing installations receive
missing defaults through migration/initialization logic, so the operation is safe to repeat.

The starter encourages a `SKILL.md` with front matter and clear task instructions. Customize a
copy or create another template when your workflow needs a different structure.

## Create a template

1. Open **Templates** and choose to create one.
2. Select a scope you are permitted to manage.
3. For a group template, select a group where you are owner or administrator.
4. Add the reusable name, description, content, and manifest defaults.
5. Save and confirm it appears to its intended audience.

Changing or deleting a group/global template requires the same management role. Existing skills
created from a template remain independent records.

## Create a skill from a template

1. Find a visible template.
2. Choose the create-from-template action.
3. Provide the new skill's name and unique slug.
4. Review the generated content and manifest.
5. Save.

The result is always a private skill owned by the user who performed the action; using a group or
global template does not make the resulting skill shared.
