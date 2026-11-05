--!strict
-- Home with the composer placed: bottom.
local home = require("@fixtures/home")
local f = table.clone(home :: any)
f.composer_place = "bottom"
return f
