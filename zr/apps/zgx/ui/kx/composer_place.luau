--!strict
-- Where a new chat's composer sits (Settings › Appearance › New chat
-- composer): "center" mid-page and opened out, "bottom" docked at the foot
-- as in a session, "slim" mid-page as one line until you write more.
--
-- One place for it: `ui.persist` keeps a value per file that asks for it,
-- so Settings and the composer each asking for "kx.composer.place" got two
-- values, and the composer never saw what Settings chose. Both require this.

return ui.persist("kx.composer.place", "center")
