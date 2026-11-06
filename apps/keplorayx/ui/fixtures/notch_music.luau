--!strict
-- The notch opened on its Music tab, a track playing.
local common = require("@fixtures/common")
return {
	projects = common.projects,
	agents = common.agents,
	threads = {},
	attention = {},
	expanded = true,
	media = { app = "Spotify", title = "Weightless", artist = "Marconi Union", playing = true, position = 100, duration = 238, at = os.time(), cover = "", tint = "#e8a6c4" },
}
