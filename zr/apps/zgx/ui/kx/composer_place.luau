--!strict
-- Where a new chat's composer sits (Settings › Appearance › New chat
-- composer): "center" mid-page and opened out; "bottom" the same composer,
-- opened out, at the foot; "slim" at the foot as one line until you write
-- more, as in a session.
--
-- One place for it: `ui.persist` keeps a value per file that asks for it,
-- so Settings and the composer each asking for "kx.composer.place" got two
-- values, and the composer never saw what Settings chose. Both require this.

return ui.persist("kx.composer.place", "center")
