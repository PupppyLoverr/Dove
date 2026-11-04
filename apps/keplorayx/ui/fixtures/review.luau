--!strict
-- The thread with its Changes panel open.
local thread = require("@fixtures/thread")
local f = table.clone(thread :: any)
f.changes_open = true
return f
