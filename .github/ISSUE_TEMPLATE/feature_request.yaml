---
name: Feature request
about: Suggest an idea for this project
title: ''
labels: ''
assignees: ''

---

name: Feature request
description: Suggest an idea or improvement for Project Blaizer
title: "[Feature] "
labels: ["enhancement"]
body:
  - type: markdown
    attributes:
      value: |
        Thanks for the idea! Before you submit:
        - Search the existing issues and the `CHANGELOG.md` in case it is already requested or done.
        - **Do not attach game data** (files from your Motorhead install, disc images, ripped music or videos).
          Screenshots of the original game as a reference are fine.
        - One feature per issue, please.

  - type: dropdown
    id: area
    attributes:
      label: Area
      description: Which part of the project does this touch?
      options:
        - Gameplay
        - Physics / handling
        - AI opponents
        - Graphics
        - Audio
        - Menus / HUD
        - Input (keyboard, gamepad, wheel)
        - Editors / tools (Cine, World, textures)
        - Installer
        - Performance
        - Settings / saves
        - Other
    validations:
      required: true

  - type: dropdown
    id: mode
    attributes:
      label: Mode
      description: >
        Classic follows the original game's look and feel, so ideas that change it are best
        suggested as an option or for Remaster.
      options:
        - Classic
        - Remaster
        - Both
        - Not related to graphics
    validations:
      required: true

  - type: textarea
    id: problem
    attributes:
      label: Problem or motivation
      description: What is missing, annoying or limiting today? Why would this be useful?
      placeholder: "When I ..., I can't ... / It would be nice if ..."
    validations:
      required: true

  - type: textarea
    id: solution
    attributes:
      label: Proposed solution
      description: What would you like to happen? Describe what you would see, hear or feel as a player.
    validations:
      required: true

  - type: textarea
    id: alternatives
    attributes:
      label: Alternatives considered
      description: Other approaches or workarounds you thought about, if any.
    validations:
      required: false

  - type: textarea
    id: context
    attributes:
      label: Additional context
      description: Mockups, screenshots, links, references to other games, anything that helps.
    validations:
      required: false

  - type: checkboxes
    id: checks
    attributes:
      label: Checklist
      options:
        - label: I searched the existing issues and this has not been requested yet.
          required: true
        - label: I did not attach any game data or copyrighted files.
          required: true
