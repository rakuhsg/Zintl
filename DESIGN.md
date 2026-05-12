# [DRAFT] DESIGN.md

## Project Overview

Zintl is a GPU-first, framework-agnostic desktop app runtime for JavaScript and WebGPU, powered by Deno.

The project is not trying to recreate a browser or provide full HTML/CSS compatibility. It is a GPU-native application runtime for desktop apps, especially canvas-centric and creative tools.

The reference application is a local-first, user-sovereign Figma-like design tool. Use this as the guiding product when making architecture decisions.

## Core Principles

### GPU-first

Rendering, composition, animation, canvas surfaces, and visual effects should be designed around GPU-native primitives.

Prefer:

- retained scene graphs
- explicit compositor layers
- WebGPU surfaces
- Vello vector rendering
- immutable render snapshots
- predictable frame scheduling

Avoid:

- CPU-first raster pipelines unless needed
- browser-style DOM/CSS compatibility work unless explicitly requested
- heavyweight layout or rendering systems that obscure GPU ownership

### Framework-agnostic

The runtime must not assume React, Vue, Solid, Svelte, or any specific UI framework.

Framework integrations should be adapters on top of primitive components and scene graph APIs.

Do not bake a React-like ownership model into the runtime core.

### Browserless, not browser-compatible

Zintl should provide a familiar JavaScript/WebGPU development experience, but it is not a browser engine.

Do not implement browser compatibility for its own sake.

Avoid adding large DOM, CSS, HTML, or web layout compatibility layers unless there is a clear runtime-level need.

### GPU-first UI

The main app UI should be rendered by GPU.

Native widgets are also available but are not the default layout system.

Use Vello for:

- sidebars
- panels
- tabs
- toolbars
- inspector UIs
- some other widgets

Use native widgets only when they provide clear platform value:

- popovers
- overlays
- dialogs
- complex text input
- IME-sensitive controls
- accessibility
- platform-specific controls
- OS integration points

### Deno is the script runtime

Deno is the preferred JavaScript runtime because it is Web-platform-oriented and Rust-friendly.

Use Deno for:

- JavaScript and TypeScript execution
- Web APIs
- WebGPU access
- module loading
- permissions
- plugin scripting
- async I/O

## Embeddable WebGPU Surface

Independent view region used for WebGPU swapchain surfaces.
