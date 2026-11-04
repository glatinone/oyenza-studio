# Floaty brand guidelines

How to name, write about and present Floaty. Use this page for any text the app shows, the README, release notes, store copy and anything said about the product in public.

## Overview and positioning

**Floaty is a fork of [Oyenza Studio](https://github.com/lucacadalora/oyenza-studio) by lucacadalora (MIT), not an independent product.** Almost all of the app is the work of lucacadalora and the Oyenza Studio contributors. Every public text (README, about text, release notes, posts) says so first, credits them, links upstream and recommends the official installer. Only the additions in the README's "What this fork adds" are described as this fork's work, and general improvements are offered back upstream as pull requests. The positioning below is for this personal build and never replaces that credit.

**Floaty is an AI desktop companion.** It lives at the top of your screen as a small floating island, stays out of the way, and opens when you point at it: what's playing, what's next, what's left of your AI plans, files from your phone, a timer, a note.

- **Category:** AI Desktop Companion.
- **Promise:** a little intelligence, everywhere on your desktop, without opening another app.
- **Built on:** Floaty is built on [Oyenza Studio](https://github.com/lucacadalora/oyenza-studio) by lucacadalora (MIT). Say so plainly in the README and in any "about" text. Never present the upstream work as Floaty's own origin story.
- **Not:** a chatbot window, a launcher that takes the whole screen, or a toy. Floaty is small, always there and quiet until it is useful.

## Personality and tone of voice

Floaty is **minimal, approachable, intelligent, and playful but not childish.** Reference points: Cursor, Linear, Raycast.

Write like this:

- **Short and concrete.** Say what happens: "Floaty is up to date", "Allow Floaty on private networks". One idea per sentence.
- **Calm.** No exclamation marks in the app, no hype words ("revolutionary", "magic", "supercharge"), no countdowns or pressure.
- **Helpful first.** A notice says what happened and what to do next, in that order.
- **Warm, not cute.** A light touch is fine in empty states; baby talk, emoji in UI text and jokes in error messages are not.
- **Honest.** Never claim a feature that isn't built. If something needs a sign-in, a key or a permission, say so.

Avoid: "Oops!", "Uh-oh", "We're sorry for the inconvenience", "Click here", "AI-powered" as filler.

## Naming conventions

- The product is **Floaty**. One word, capital F, no "the" ("Open Floaty", not "Open the Floaty").
- Possessive: **Floaty's** ("Floaty's settings").
- Do not translate, abbreviate, hyphenate or pluralise the name (no "Floaties", "FLTY", "Floaty-app", "FloatyAI").
- Module names are **Floaty + one plain English noun**, both capitalised: Floaty Bar, Floaty Chat. Never "FloatyBar", "Floaty-Chat" or "floaty chat" in running text.
- After the first mention in a page, a module may be called by its noun alone when it is clear ("the Bar").
- Internal technical names stay as they are and are never shown to people as the brand: the `poni` crates and `poni.exe`, the `Poni.*` window classes, `%APPDATA%\Poni` and `%LOCALAPPDATA%\Poni`, the AUMID, the Run value, the update feed and the `OyenzaStudio/` User-Agent. Renaming them would break settings, updates and autostart.
- Names that belong to other products keep their names: **Oyenza** (the account, oyenza.app, Oyenza Inference, Studio Pro), **Oyenza Studio Recorder** (a separate app), and **Oyen** (the cat character).

## Product architecture

| Layer | Name | Status |
| --- | --- | --- |
| Main brand | **Floaty** | Shipping (this app) |
| Floating interface (the island) | **Floaty Bar** | Planned name for the existing island; not used in the app yet |
| AI conversation | **Floaty Chat** | Planned, not built |
| Computer interaction | **Floaty Actions** | Planned, not built |
| Screen intelligence | **Floaty Vision** | Planned, not built |
| Quick commands | **Floaty Shortcuts** | Planned, not built |
| Later | **Floaty Memory**, **Floaty Voice**, **Floaty Workspace** | Ideas only |

Planned names are a naming system, not a roadmap promise. Do not show them in the app, in screenshots or in release notes until the feature exists and works.

## Wordmark

- **In text, always "Floaty"** (capital F): sentences, headings, menus, window titles, notices, metadata (`ProductName`, `FileDescription`), store listings.
- **As the visual mark, "floaty"** (all lowercase): only where the name stands alone as a logo, such as the README hero, a splash, a website header, the app icon lockup or a social avatar.
- Never mix the two in one place ("floaty" inside a sentence, or "Floaty" in the logo position).
- Set the wordmark in the app's own typeface, one weight, no outlines, gradients, drop shadows or stretching. Keep clear space around it of at least the height of the "f".
- Module names never use the lowercase mark: it is "Floaty Chat", not "floaty chat".

## Tagline

- **Primary:** "A little intelligence, everywhere."
- **Descriptive line:** "Your AI companion that lives on your desktop."

Use the primary tagline under the wordmark and as the one-line pitch. Use the descriptive line where a reader needs to know what the product is (store listing, README intro, a link preview). Do not rewrite either one; do not add a third.

## Visual direction

**Minimal, clean, lightweight, modern, desktop-native.**

- **Minimal:** one island, few elements, generous spacing. If a screen needs a legend, it has too much on it.
- **Clean:** flat surfaces, one accent colour at a time, text that reads at a glance.
- **Lightweight:** light on CPU, memory and download size is part of the brand (see `AGENTS.md`). Motion is short and springy, never decorative loops.
- **Modern:** current system typography, rounded geometry that matches the island's shape, dark by default.
- **Desktop-native:** follows Windows (and macOS) conventions for menus, tray, settings and notifications. It should feel like part of the OS, not a web page in a window.

The current app icon and the Oyen character come from Oyenza Studio. They are upstream brand assets, not covered by the MIT license (see the README's License section), and need to be replaced with Floaty's own art before Floaty is distributed.

## Introducing future features

1. **Build first, name second.** A name goes into the app only when the feature ships. Until then it lives in this page's table as "Planned".
2. **Pick the layer.** If the feature fits an existing layer (Bar, Chat, Actions, Vision, Shortcuts), it is part of that module and needs no new name.
3. **New module only when it is a new layer.** Then name it "Floaty" + one plain, concrete English noun that says what it does (Memory, Voice). No invented words, no acronyms, no version numbers in the name.
4. **Check the name** against this page: it must read well in a menu ("Open Floaty Voice"), in a sentence and in a notice title, and must not collide with an existing module or a well-known product.
5. **Add it here** (table above, with its status) in the same change that ships it, and update the README.
6. **Keep internal names stable.** A new module may have any code name; it does not rename crates, folders, registry values or window classes.

## Before any commercial launch

"Floaty" and every sub-name (Floaty Bar, Chat, Actions, Vision, Shortcuts, Memory, Voice, Workspace) have not been cleared. Before selling, publishing to a store or marketing Floaty, check trademark registrations in the target markets (at least Indonesia's DJKI, the US USPTO and the EU EUIPO) and domain and handle availability, and replace the upstream Oyenza Studio icon and Oyen art.
