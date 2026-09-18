# The add and edit forms are one map, not a stepped sequence

- **Status:** Accepted
- **Date:** 2026-09-17
- **Supersedes:** the five-step `Ctrl+A` sequence and the single-field `Ctrl+E` editor described in #31 stories 24 and 26

## Context

The add flow was a stepper: `Ctrl+A` opened one field at a time — Alias, Host,
Port, Key, Folder — Enter settled each and advanced, and the last Enter wrote
the Connection. The edit flow was separate: `Ctrl+E` put a single field on the
line, `←`/`→` cycled it, and Enter wrote immediately.

Three defects fell out of that shape.

**The user could not see the shape of what they were filling.** One field per
screen means the user holds the remaining work in their head. There is no
answer on screen to "how many of these do I still owe?", and the fields that
were already answered scrolled away into `◇` history lines.

**The `User` field was unreachable.** The spec named five steps and `user` was
not one of them, so every Connection added through the flow carried an empty
`user` and rendered as `(@192.168.31.7:22)`. `sshm add --user` had always
accepted one; the interactive path simply could not ask. This was a gap in the
spec, not a decision.

**Edit committed on every Enter.** Correcting two fields wrote
`connections.json` twice, and there was no way to abandon halfway — the first
change was already durable while the second was still being typed.

## Decision

**One form map, used by both commands.** Six rows — Alias, Host, User, Port,
Key, Folder — each carrying a glyph that states its condition, then a
separator, then a `▶` submit row. `Ctrl+A` opens the map blank; `Ctrl+E`
opens it seeded from the selected Connection.

The glyph set is ordinal and colour-independent: `✓` valid, `●` changed from
what is stored, `○` required and empty, `!` will not validate, `·` optional
and empty. The cursor row wears `◆`.

**The draft is live.** Every keystroke writes straight into the field under
the cursor. There is no separate input line that must be settled before the
row stops lying — the row *is* the draft. Normalisation (trim, the port's
empty-to-`22`) is deferred to the submit, so the map never shows a value the
user did not type.

**Movement is never refused.** `↑`/`↓` walk the rows, `Tab`/`Shift-Tab`
travel with wrap, and `Enter` advances off any field — including one that
will not validate. The only gate in the whole form is the `▶` row.

**The `▶` row is the single commit point.** It is dim until the form is
acceptable, and the refusal line names every offending field with its reason.

## Why movement is never refused

This is the load-bearing choice, and the one most likely to be relitigated.

The obvious alternative is the stepper's discipline: Enter on an invalid field
refuses, the user stays put, the reason appears. It fires the error at the
moment of the mistake, which teaches faster.

It was rejected because it re-creates the complaint that motivated the
redesign. "You go and cycle through all of them" is what the stepper felt
like; refusing Enter is what makes a field a trap. On a map where all six
fields are visible at once, the `!` glyph and the dim `▶` row already carry
the whole error signal — refusing the move adds no information, only
friction. The user can always see exactly what is wrong and simply cannot be
stuck.

The consequence worth naming: a user can type an invalid port and advance past
it without being stopped. That is acceptable because the invalid field stays
`!`, the button stays dim, and the refusal line on the submit attempt names
the field and its reason. Nothing is silently wrong; the failure is deferred,
never lost.

## Departure from #31 story 26

Story 26 asks that an edit "change exactly one field". Under the map, one
save can carry several changed fields.

This is deliberate. The one-field rule was a consequence of committing per
Enter, not a goal in itself. Keeping it would mean keeping the double-write.
Changed fields wear `●` so the save is reviewable before it happens, and
`▶ Save changes to <alias>` names the target so there is no ambiguity about
what is being written.

## Consequences

- **The frame's height is a hard budget.** `VISIBLE_ROWS = 8` and the map is
  exactly 6 fields + rule + button. The error line therefore *replaces* the
  rule row rather than adding a line: the rule is decoration, the refusal is
  not, and `!` at that position still separates the fields from the button.
  Any future row in the map must be paid for by removing one.
- **The list is hidden while the map is open.** The map needs all eight rows.
  The user picked their connection before pressing the chord, so the list is
  not needed behind it.
- **A new theme tier.** `fg_placeholder` (`Indexed(240)`) sits *below* the
  informational contrast floor at ≈2.2:1, deliberately, because a
  placeholder that matches the label's colour is indistinguishable from a
  label. It is exempt from the contrast-floor test for the same reason
  `border` is: it carries no state. Whether a field needs filling is said by
  its glyph, not by its placeholder.
- **The port message was shortened** to `port must be 1–65535 (e.g. 22)` so
  that all three possible problems plus the example fit in 71 columns.
- **Both flows are now one code path.** A change to the map is a change to
  add and edit simultaneously, which is the point — but it also means a bug
  in the map is a bug in both.
