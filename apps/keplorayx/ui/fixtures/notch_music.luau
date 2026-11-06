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
	tab = "music",
	-- (What a chat opened in the island shows.)
	entries = thread.entries,
	media = {
		app = "Spotify",
		title = "Fallen Angel",
		artist = "Jennie",
		playing = true,
		position = 64,
		duration = 218,
		at = os.time(),
		cover = "assets/art/plate-3-sunrise.jpg",
		tint = "#f0b7a2",
	},
}
