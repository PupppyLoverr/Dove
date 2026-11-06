--!strict
-- The notch opened on its Music tab, a track playing.
local common = require("@fixtures/common")
local thread = require("@fixtures/thread")
return {
	projects = common.projects,
	agents = common.agents,
	threads = common.threads,
	attention = {},
	expanded = true,
	-- (What a chat opened in the island shows.)
	entries = thread.entries,
	media = { app = "Spotify", title = "Weightless", artist = "Marconi Union", playing = true, position = 100, duration = 238, at = os.time(), cover = "", tint = "#e8a6c4" },
}
