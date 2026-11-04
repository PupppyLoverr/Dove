--!strict
-- The system's display settings, as states: `prefers.reduced_motion:get()`
-- (tracked) or `:peek()`. Read here rather than from `ui.prefers`, whose
-- type definitions give no `peek`; this is the one place that says what
-- they are.

export type Pref = {
	get: (self: Pref) -> boolean,
	peek: (self: Pref) -> boolean,
}

local system = ui.prefers :: any

return {
	reduced_motion = system.reduced_motion :: Pref,
	more_contrast = system.more_contrast :: Pref,
	reduced_transparency = system.reduced_transparency :: Pref,
}
