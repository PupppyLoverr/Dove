--!strict
-- Keplorayx's helpers for a plugin (after Zeron's plugins-sdk/zeron.luau,
-- MIT). A copy sits in every plugin Keplorayx makes (an agent's
-- create_plugin, or the catalog's). Use it from init.luau:
--
--   local kx = require("./kx")("my-plugin")   -- the plugin's own id
--
-- Where your plugin shows, by an element's id:
--
--   kx.place { at = "#sidebar", after = Badge }
--   kx.place { at = "#composer", before = Hint }
--   kx.place { at = "#home", replace = Welcome }      (needs "ui.change")
--
-- `Badge` is a node, or a function that builds one.
--
-- Doing things (what each needs, from the manifest's permissions):
--
--   kx.open(thread_id)            open a thread                   navigate
--   kx.compose(text)              text into the composer          compose
--   kx.threads()                  your threads, as they change    sessions.read
--   kx.thread()                   the open thread, or nil         sessions.read
--   kx.now()                      the time, to the minute         (always)
--   kx.toast(title, body?)        a short note in the window      (always)
--   kx.store(key, default)        kept between runs               (always)
--   kx.every(seconds, fn)         now and then, 30 s at least     (always)
--
-- Settings, declared in kx-plugin.json and set by you in Settings ›
-- Plugins (switch, choice, text, number):
--
--   local length = kx.setting("length")   :get() follows it, :peek() doesn't
--   kx.settings()                         every value, by id
--   kx.on_setting(function(id, value, was) … end)   each change you make

type Box = { get: (any) -> any, peek: (any) -> any }

return function(id: string)
	local M = {}

	-- ── Where it shows ──────────────────────────────────────────────────
	type Place = { at: string, before: any?, after: any?, replace: any? }
	local function built(n: any): any
		if type(n) == "function" then
			return n()
		end
		return n
	end
	function M.place(p: Place)
		ui.contribute {
			target = p.at,
			before = if p.before ~= nil then built(p.before) else nil,
			after = if p.after ~= nil then built(p.after) else nil,
			replace = if p.replace ~= nil then built(p.replace) else nil,
		}
	end

	-- ── Doing things ────────────────────────────────────────────────────
	function M.open(thread: string)
		ui.invoke("kx.open", thread)
	end
	local composer: any = nil
	function M.compose(text: string)
		composer = composer or ui.shared("ray.compose", "")
		composer:set(text)
	end
	local threads: any = nil
	function M.threads(): { any }
		threads = threads or ui.shared("kx.threads", {})
		return threads:get() or {}
	end
	local open_thread: any = nil
	function M.thread(): any
		open_thread = open_thread or ui.shared("kx.thread", nil)
		return open_thread:get()
	end
	local now: any = nil
	function M.now(): any
		now = now or ui.shared("kx.now", nil)
		return now:get()
	end
	local toast: any = nil
	local said = 0
	function M.toast(title: string, body: string?)
		toast = toast or ui.shared("kx.plugin_toast", nil)
		said += 1
		toast:set({ plugin = id, title = title, body = body, n = said })
	end

	-- Kept between runs, under the plugin's id.
	function M.store(key: string, default: any): any
		local ok, state = pcall(ui.persist, `{id}.{key}`, default)
		if ok then
			return state
		end
		return ui.state(`{id}.{key}`, default)
	end

	-- Every `seconds`, 30 at least; what fails in it is said, and it goes
	-- on. Returns a function that stops it.
	function M.every(seconds: number, run: () -> ()): () -> ()
		return ui.every(math.max(30, seconds) * 1000, function()
			local ok, why = pcall(run)
			if not ok then
				print(`{id}: an every() run failed: {why}`)
			end
		end)
	end

	-- ── Settings ────────────────────────────────────────────────────────
	local all = ui.shared("kx.plugin_settings", {} :: any)
	local function mine(peek: boolean): { [string]: any }
		local v = if peek then all:peek() else all:get()
		return (v or {})[id] or {}
	end
	function M.settings(): { [string]: any }
		return table.clone(mine(false))
	end
	function M.setting(key: string): Box
		return {
			get = function(_self: any): any
				return mine(false)[key]
			end,
			peek = function(_self: any): any
				return mine(true)[key]
			end,
		}
	end
	local listeners: { (string, any, any) -> () } = {}
	local seen: { [string]: any }? = nil
	function M.on_setting(listen: (id: string, value: any, was: any) -> ())
		table.insert(listeners, listen)
		if seen ~= nil then
			return
		end
		seen = table.clone(mine(true))
		ui.effect(function()
			local now_values = mine(false)
			local was = seen or {}
			seen = table.clone(now_values)
			for key, value in now_values do
				if was[key] ~= value then
					for _, l in listeners do
						local ok, why = pcall(l, key, value, was[key])
						if not ok then
							print(`{id}: a setting's listener failed: {why}`)
						end
					end
				end
			end
		end)
	end

	return M
end
