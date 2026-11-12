--!strict
-- What the clock sends to buddies. src/kx/buddies.rs lists what's come due
-- in `kx.automations.due` (each `{ run, automation, buddy, title, prompt,
-- at }`); each is sent here to its buddy's chat (made if it has none), on
-- the first agent of its order that's usable, and how it went is said back
-- (`kx.automation.ran`), which is what the buddy's Activity lists. Sent
-- once: a run is begun the first time it's seen.
--
-- Loaded with the main window's scripts (screens/shell/sidebar), so it runs
-- while the window is open and only then.

local store = require("@app/kx/buddies/store")

local due = ui.shared("kx.automations.due", {} :: { any })
local begun: { [string]: boolean } = {}

local function ran(run: string, status: string, note: string?, used: any)
	pcall(ui.invoke, "kx.automation.ran", {
		run = run,
		status = status,
		note = note,
		chat = used and used.chat,
		harness = used and used.harness,
		model = used and used.model,
	})
end

local function send(d: any)
	local b = store.by_id(d.buddy)
	if not b then
		ran(d.run, "failed", "Its buddy is gone.", nil)
		return
	end
	local buddy: store.Buddy = b
	local harness, _, why = store.pick(buddy)
	if not harness then
		ran(d.run, "failed", why, nil)
		return
	end
	-- (Said in the message which routine this is, and, when it isn't on its
	-- first choice, on what it ran and why: the chat is where that's read.)
	local head = `[Keplorayx routine: {d.title or "untitled"}]`
	if why then
		head ..= `\n[Keplorayx: {why}]`
	end
	store.send(buddy, `{head}\n\n{d.prompt or ""}`, function(ok: boolean, note: string?, used: any)
		ran(d.run, if ok then "sent" else "failed", note, used)
	end)
end

ui.effect(function()
	local waiting: { [string]: boolean } = {}
	for _, d in due:get() or {} do
		if type(d) == "table" and type(d.run) == "string" then
			waiting[d.run] = true
			if not begun[d.run] then
				begun[d.run] = true
				ui.after(0, function()
					local ok, err = pcall(send, d)
					if not ok then
						ran(d.run, "failed", tostring(err), nil)
					end
				end)
			end
		end
	end
	for id in begun do
		if not waiting[id] then
			begun[id] = nil
		end
	end
end)

return {}
