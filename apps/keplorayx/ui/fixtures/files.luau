--!strict
-- The thread with the panel open on files.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.panel_tab = "files"
return f
