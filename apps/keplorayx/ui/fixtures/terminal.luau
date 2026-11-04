--!strict
-- The thread with its panel on Terminal.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.panel_tab = "terminal"
return f
