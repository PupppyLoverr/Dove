--!strict
-- The launcher (Alt+Space): Zeron's command palette (ds/command_palette,
-- vendored, MIT) in a window of its own over everything. One field finds
-- an app, a window open now, a file, something you copied, a thread, a
-- project, a setting or a command; works out a sum or a unit as you type;
-- searches the web; and asks an agent, its answer streaming in place
-- (Enter on "Ask Keplorayx"). Escape, or a click elsewhere, hides it.
-- What it learned from Raycast, Spotlight, PowerToys and the rest:
-- docs/design/launcher-study.md. The host side: src/launcher.rs.

ui.theme.strict("warn")
local kx = require("@app/kx")
kx.setup()

local CommandPalette = require("@ds/command_palette")
local Markdown = require("@ds/markdown")
local store = require("@app/kx/store")
local calc = require("@app/kx/calc")
-- (A new thread's agent and model: what the last new chat used, as Zeron's
-- composer keeps them.)
local ray = { project = ui.state("launcher.project", ""), model = ui.state("launcher.model", nil :: string?), effort = ui.state("launcher.effort", nil :: string?) }

local open = ui.state("launcher.open", store.fixture ~= "")
-- "find" (the palette) or "answer" (an agent's reply to what you asked).
local mode = ui.state("launcher.mode", "find")
local asked = ui.state("launcher.asked", "")
local failed = ui.state("launcher.failed", "")

local function hide()
	open:set(false)
	mode:set("find")
	if store.fixture == "" then
		pcall(ui.invoke, "launcher.hide", "hide()")
	end
end

-- Summoned (the key, the tray, the notch): the field again, empty.
local summoned: State<number> = ui.shared("launcher.summoned", 0)
-- What plays (src/media.rs): played, paused and skipped from the list.
local media: State<any> = if store.fixture ~= ""
	then ui.state("fx.media", { title = "Weightless", artist = "Marconi Union", playing = true })
	else ui.shared("kx.media", nil)
-- What you picked lately, first again the next time.
local recent = ui.persist("launcher.recent", {} :: { string })
ui.effect(function()
	if summoned:get() > 0 then
		mode:set("find")
		open:set(true)
	end
end)

-- The palette closed by Escape or a row done: the window goes too, unless
-- it's answering.
ui.effect(function()
	if not open:get() and mode:peek() == "find" and summoned:peek() > 0 then
		pcall(ui.invoke, "launcher.hide", "palette closed")
	end
end)

-- A click in another app: away it goes.
local ok_active, active = pcall(function(): any
	return (ui :: any).window.active
end)
-- (Only once it has had the focus since it was summoned: Windows may hold
-- the foreground for a moment, and it mustn't hide before it shows.)
local had_focus = false
ui.effect(function()
	summoned:get()
	had_focus = false
end)
if ok_active and active ~= nil then
	ui.effect(function()
		local now = active:get()
		if now == true then
			had_focus = true
		elseif now == false and had_focus and summoned:peek() > 0 then
			had_focus = false
			hide()
		end
	end)
end

-- ── Doing things ────────────────────────────────────────────────────────

local function run(kind: string, target: string): () -> ()
	return function()
		if store.fixture == "" then
			ui.invoke("launcher.run", { kind = kind, target = target })
		end
	end
end

local function copy(text: string): () -> ()
	return function()
		pcall(ui.invoke, "launcher.copy", text)
		pcall(ui.invoke, "launcher.hide")
	end
end

local function to_main(thread: string?): () -> ()
	return function()
		pcall(ui.invoke, "launcher.main", thread)
	end
end

-- Asks a new thread, with the composer's agent, model and project, and
-- shows its answer here as it comes.
local function ask(q: string): () -> ()
	return function()
		asked:set(q)
		failed:set("")
		mode:set("answer")
		local project = ray.project:peek()
		store.send(q, {
			project = if project ~= "" then project else nil,
			model = ray.model:peek(),
			effort = ray.effort:peek(),
			access = "supervised",
		}):after(function() end, function(why: any)
			failed:set(tostring(if type(why) == "table" then why.message else why))
		end)
	end
end

local function web(q: string): string
	return "https://duckduckgo.com/?q=" .. string.gsub(q, "[^%w%-_%.~]", function(c: string): string
		return string.format("%%%02X", string.byte(c))
	end)
end

-- ── What it lists ───────────────────────────────────────────────────────

local SECTION: { [string]: string } = {
	app = "Applications",
	window = "Windows",
	clip = "Copied",
	file = "Files",
}
local ICON: { [string]: string } = {
	app = "widget",
	window = "window-restore",
	clip = "clipboard",
	file = "document",
}

-- A row the host found, as the palette's.
local function found_item(f: any): any
	local kind: string = f.kind
	local item: { [string]: any } = {
		id = f.id,
		label = f.label,
		section = SECTION[kind] or "Results",
		scope = if kind == "file" then "files" else "commands",
		icon = ICON[kind],
		hint = if kind == "file" or kind == "clip" then nil else f.sub,
		keywords = f.sub,
		verb = if kind == "clip" then "Copy" elseif kind == "window" then "Switch to" else "Open",
		run = if kind == "clip" then copy(f.target) else run(kind, f.target),
	}
	if kind == "file" or kind == "app" then
		item.alt = { label = "Show in folder", run = run("reveal", f.target) }
	elseif kind == "clip" then
		item.alt = { label = "Ask about it", run = ask(f.target) }
	end
	return item
end

-- A promise of rows: the host's, after what the typing itself answers
-- (a sum, a unit, the web, an agent).
local function searched(q: string, only: string?): any
	local own: { any } = {}
	local last: { any } = {}
	if not only and q ~= "" then
		local v = calc.eval(q)
		local conv, unit = calc.convert(q)
		if v then
			local s = calc.format(v)
			table.insert(own, { id = "calc", label = `= {s}`, section = "Answer", icon = "sparkle", verb = "Copy", hint = q, run = copy(s) })
		elseif conv then
			local UNIT: { [string]: string } = { c = "°C", f = "°F", k = "K" }
			local s = `{calc.format(conv)} {UNIT[unit :: string] or unit}`
			table.insert(own, { id = "calc", label = `= {s}`, section = "Answer", icon = "sparkle", verb = "Copy", hint = q, run = copy(s) })
		end
		table.insert(last, { id = "ask", label = `Ask Keplorayx “{q}”`, section = "Ask", icon = "ray", verb = "Ask", run = ask(q) })
		table.insert(last, { id = "web", label = `Search the web for “{q}”`, section = "Ask", icon = "globe", verb = "Search", run = run("url", web(q)) })
	end
	if store.fixture ~= "" then
		return ui.sleep(0):after(function()
			return table.move(last, 1, #last, #own + 1, own)
		end)
	end
	local p = ui.call("launcher.search", { q = q, only = only })
	return {
		after = function(_, done: (any) -> (), fail: ((any) -> ())?)
			-- What the typing answers, at once; the host's after, in its place.
			done(table.move(last, 1, #last, #own + 1, table.clone(own)))
			p:after(function(rows: any)
				local out = table.clone(own)
				for _, f in rows or {} do
					table.insert(out, found_item(f))
				end
				done(table.move(last, 1, #last, #out + 1, out))
			end, function(why: any)
				done(table.move(last, 1, #last, #own + 1, table.clone(own)))
				if fail then
					fail(why)
				end
			end)
		end,
	}
end

local function page_of(id: string, title: string, only: string, placeholder: string): () -> any
	return function()
		return {
			id = id,
			title = title,
			placeholder = placeholder,
			search = function(q: string): any
				return searched(q, only)
			end,
		}
	end
end

-- Snippets: text you keep, put on the clipboard by name. Quick links: a
-- site's search, the words typed after it.
local snippets = ui.persist("launcher.snippets", {} :: { { name: string, text: string } })
local LINKS = {
	{ name = "GitHub", url = "https://github.com/search?q=" },
	{ name = "Google", url = "https://www.google.com/search?q=" },
	{ name = "YouTube", url = "https://www.youtube.com/results?search_query=" },
	{ name = "Wikipedia", url = "https://en.wikipedia.org/w/index.php?search=" },
	{ name = "MDN", url = "https://developer.mozilla.org/en-US/search?q=" },
	{ name = "crates.io", url = "https://crates.io/search?q=" },
	{ name = "npm", url = "https://www.npmjs.com/search?q=" },
}

local function link_page(l: { name: string, url: string }): () -> any
	return function()
		return {
			id = `link:{l.name}`,
			title = l.name,
			placeholder = `Search {l.name}`,
			search = function(q: string): any
				local items = if q == ""
					then {}
					else { { id = "go", label = `Search {l.name} for “{q}”`, section = l.name, icon = "globe", verb = "Search", run = run("url", l.url .. string.sub(web(q), #"https://duckduckgo.com/?q=" + 1)) } }
				return ui.sleep(0):after(function()
					return items
				end)
			end,
		}
	end
end

local function snippet_page(): any
	return {
		id = "snippets",
		title = "Snippets",
		placeholder = "Find a snippet",
		items = function(): { any }
			local out: { any } = {}
			for i, s in snippets:get() do
				table.insert(out, {
					id = `snippet:{i}`,
					label = s.name,
					keywords = s.text,
					section = "Snippets",
					icon = "note-text",
					verb = "Copy",
					run = copy(s.text),
					alt = { label = "Delete", run = function()
						local next = table.clone(snippets:peek())
						table.remove(next, i)
						snippets:set(next)
					end },
				})
			end
			table.insert(out, {
				id = "snippet:new",
				label = "Keep what's copied as a snippet",
				section = "New",
				icon = "plus",
				run = function()
					local text = ui.clipboard.read()
					if type(text) == "string" and text ~= "" then
						local next = table.clone(snippets:peek())
						local name = string.sub((string.gsub(text, "%s+", " ")), 1, 40)
						table.insert(next, { name = name, text = text })
						snippets:set(next)
					end
				end,
			})
			return out
		end,
		empty = { title = "No snippets yet", hint = "Copy some text, then keep it here." },
	}
end

local SYSTEM = {
	{ "Lock the screen", "lock", "key-minimalistic" },
	{ "Task Manager", "task_manager", "chart" },
	{ "Take a screenshot", "screenshot", "screen-capture" },
	{ "Downloads", "downloads", "download" },
	{ "Recycle Bin", "recycle_bin", "trash-bin-minimalistic" },
	{ "Wi-Fi settings", "ms-settings:network-wifi", "global" },
	{ "Bluetooth settings", "ms-settings:bluetooth", "settings" },
	{ "Display settings", "ms-settings:display", "monitor" },
	{ "Sound settings", "ms-settings:sound", "volume-loud" },
	{ "Apps settings", "ms-settings:appsfeatures", "widget" },
	{ "Windows Update", "ms-settings:windowsupdate", "refresh" },
}

local MARK: { [string]: string } = {
	claude_code = "claude",
	codex = "openai",
	cursor = "cursor",
	grok = "grok",
	devin = "devin",
	open_code = "opencode",
	antigravity = "antigravity",
	pi = "pi",
	hermes = "hermes",
}

local function root_items(): { any }
	local out: { any } = {
		{ id = "kx:new", label = "New thread", section = "Keplorayx", icon = "pen-new-square", pinned = true, run = function()
			pcall(ui.invoke, "launcher.main", "")
		end },
		{ id = "kx:open", label = "Open Keplorayx", section = "Keplorayx", icon = "orbit", run = to_main(nil) },
		{ id = "kx:clips", label = "Clipboard history", section = "Keplorayx", icon = "clipboard-list", pinned = true, page = page_of("clips", "Clipboard history", "clip", "Find something you copied") },
		{ id = "kx:windows", label = "Switch window", section = "Keplorayx", icon = "window-restore", pinned = true, page = page_of("windows", "Switch window", "window", "Find a window") },
		{ id = "kx:snippets", label = "Snippets", section = "Keplorayx", icon = "note-text", page = snippet_page },
	}
	local m = media:get()
	if m and (m.title or "") ~= "" then
		local now = if (m.artist or "") ~= "" then `{m.title} · {m.artist}` else m.title
		local function control(what: string): () -> ()
			return function()
				pcall(ui.invoke, "media.control", what)
			end
		end
		table.insert(out, { id = "media:play", label = if m.playing then "Pause" else "Play", hint = now, keywords = "music media song track pause play", section = "Now playing", icon = if m.playing then "media-pause" else "media-play", pinned = true, verb = if m.playing then "Pause" else "Play", run = control("play") })
		table.insert(out, { id = "media:next", label = "Next track", hint = now, keywords = "music media song skip", section = "Now playing", icon = "media-next", run = control("next") })
		table.insert(out, { id = "media:previous", label = "Previous track", hint = now, keywords = "music media song back", section = "Now playing", icon = "media-previous", run = control("previous") })
	end
	for _, l in LINKS do
		table.insert(out, { id = `link:{l.name}`, label = `Search {l.name}`, section = "Quick links", icon = "link", page = link_page(l) })
	end
	for _, s in SYSTEM do
		table.insert(out, { id = `sys:{s[2]}`, label = s[1], section = "System", icon = s[3], run = run("system", s[2]) })
	end
	for _, t in store.threads:get() do
		table.insert(out, {
			id = `thread:{t.id}`,
			label = t.title,
			section = "Threads",
			mark = MARK[t.harness_id],
			scope = "sessions",
			session = { agent = MARK[t.harness_id], project = t.project },
			run = to_main(t.id),
		})
	end
	return out
end

local root = {
	id = "root",
	title = "Keplorayx",
	placeholder = "Search, ask, or work something out…",
	scopes = true,
	items = root_items,
	search = function(q: string): any
		return searched(q, nil)
	end,
}

-- ── An answer ───────────────────────────────────────────────────────────

local reply = ui.computed(function(): string
	local d = store.draft:get()
	if d.text ~= "" then
		return d.text
	end
	local last = ""
	for _, e in store.entries:get() do
		if e.kind ~= "user" and e.kind ~= "tool" and e.kind ~= "thinking" and e.text ~= "" then
			last = e.text
		end
	end
	return last
end)
local running = ui.computed(function(): boolean
	local t = store.thread:get()
	return t ~= nil and t.running
end)

local function answer(): Node
	return ui.column(kx.float_style {
		w = 640,
		rounded = "$radius.xl",
		p = "$space.4",
		gap = "$space.3",
		on_key = function(e: any): boolean
			if e.key == "Escape" then
				hide()
				return true
			end
			return false
		end,
		focusable = true,
		autofocus = true,
		ui.row {
			gap = "$space.2",
			items = "center",
			ui.column { color = "$accent", kx.Icon { name = "ray", size = 14 } },
			ui.text { asked, literal = true, weight = "$font.weight.medium", truncate = true, grow = 1 },
			ui.show(running, kx.Spinner { size = 12 }),
		},
		ui.div { h = 1, bg = "$border.subtle" },
		ui.scroll {
			max_h = 380,
			ui.show(
				function()
					return failed:get() ~= ""
				end,
				ui.text { failed, literal = true, color = "$danger", text_size = "$font.size.sm" },
				ui.show(
					function()
						return reply:get() ~= ""
					end,
					Markdown { text = function()
						return reply:get()
					end, key = "launcher" },
					ui.text { "Thinking…", literal = true, color = "$text.tertiary", text_size = "$font.size.sm" }
				)
			),
		},
		ui.row {
			gap = "$space.2",
			justify = "end",
			kx.Button { tone = "ghost", size = "sm", on_click = function()
				pcall(ui.invoke, "launcher.copy", reply:peek())
			end, "Copy" },
			kx.Button { tone = "secondary", size = "sm", on_click = function()
				local t = store.thread:peek()
				to_main(if t then t.id else nil)()
			end, "Open in Keplorayx" },
			kx.Button { tone = "ghost", size = "sm", on_click = function()
				mode:set("find")
				open:set(true)
			end, "Back" },
		},
	})
end

-- The palette's card alone (no modal: its dim would fill this window), in
-- the float variant of your theme: a card that stands without glass.
local kind = ui.theme.kind()
local float_theme = if kind == "light" or kind == "high-contrast-light" then "orbit-dawn-float" else "orbit-night-float"
return ui.column {
	w_full = true,
	h_full = true,
	items = "center",
	ui.show(
		function()
			return mode:get() == "answer"
		end,
		ui.column { pt = ui.raw(88, "level with the palette's card"), answer() },
		ui.column {
			theme = float_theme,
			pt = ui.raw(88, "where the palette's card sits"),
			ui.show(
				open,
				-- (Its card is glass made for a window's blur; here nothing
				-- is blurred under it, so a solid night stands behind it.)
				ui.column {
					w = 640,
					rounded = "$radius.xl",
					bg = "$zeron.menu.solid",
					shadow = "$shadow.lg",
					CommandPalette.Inline { open = open, root = root :: any, recent = recent },
				}
			),
		}
	),
}
