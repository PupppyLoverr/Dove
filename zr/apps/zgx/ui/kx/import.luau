--!strict
-- Keplorayx's projects from before it moved onto Zeron's engine, brought
-- across once (src/kx/import.rs reads them): each folder becomes a project
-- on this computer unless the engine already has it. Required once by
-- app.luau; it waits for the sessions to be ready, then runs.

local sessions = ui.shared("sessions", {} :: any)
local done = false

local function same(a: string, b: string): boolean
	local function norm(p: string): string
		return string.lower((string.gsub(string.gsub(p, "\\", "/"), "/+$", "")))
	end
	return norm(a) == norm(b)
end

ui.effect(function()
	local s = sessions:get() or {}
	if done or s.state ~= "ready" or not s.here then
		return
	end
	done = true
	local here = s.here
	local known = s.projects or {}
	-- (Only the main window has it: setup's window asks nothing.)
	local ok, asked = pcall(ui.call, "kx.import.projects", nil)
	if not ok then
		return
	end
	asked:after(function(list: any)
		for _, p in list or {} do
			local there = false
			for _, k in known do
				if k.path and same(k.path, p.path) then
					there = true
				end
			end
			if not there then
				ui.call("projects.create", { device = here, path = p.path, git = p.git }):after(function() end, function() end)
			end
		end
		pcall(ui.invoke, "kx.import.done")
	end, function() end)
end)

return {}
