--!strict
-- The Buddies space's pages (under the `kx` plugin name, ui/kx/pages):
--
--   page.board()   every buddy on a card (what it is, how it stands), the
--                  starter templates to make one from, and the last runs
--   page.buddy()   the one selected (store.selected): Profile (who it is,
--                  what it runs on, what it may do), Automations (what it's
--                  sent by the clock) and Activity (its runs)
--
-- A buddy's conversation is its chat on the engine, shown by the chat page
-- as any chat is: "Open chat" goes there, and the sidebar stays the buddies'.

local ds = require("@ds")
local Icon = require("@ds/icon")
local ListRow = require("@ds/list_row").ListRow
local status = require("@ds/status")
local route = require("@app/screens/shell/route")
local sessions = require("@account/sessions")
local store = require("@app/kx/buddies/store")
local Avatar = require("@app/kx/buddies/avatar")
local templates = require("@app/kx/buddies/templates")

local M = {}

M.PLUGIN = "kx"

local STANDING: { [string]: string } = {
	working = "Working",
	connecting = "Starting",
	plan = "Planning",
	input = "Waiting on you",
	approval = "Needs your approval",
	failed = "Failed",
	limited = "At its limit",
	done = "Done",
	waiting = "Waiting",
}
local function standing_words(b: store.Buddy): string
	return STANDING[store.standing(b) or ""] or (if b.chat then "Idle" else "Not started")
end
local function standing_color(b: store.Buddy): string
	local s = store.standing(b)
	if s == "input" or s == "approval" then
		return "$warning"
	elseif s == "failed" or s == "limited" then
		return "$danger"
	elseif s == "working" or s == "connecting" or s == "plan" then
		return "$accent"
	elseif s == "done" then
		return "$success"
	end
	return "$text.tertiary"
end

local function go_board()
	store.selected:set(nil)
	route.plugin_page(M.PLUGIN, "buddies")
end
local function go_buddy(id: string)
	store.selected:set(id)
	route.plugin_page(M.PLUGIN, "buddy")
end

-- A new buddy, from a template or bare, and its page.
local function create(t: templates.Template?)
	local record: { [string]: any } = if t
		then { name = t.name, role = t.role, instructions = t.instructions, look = t.look, permissions = "draft", models = {} }
		else { name = "New buddy", role = "", instructions = "", permissions = "draft", models = {} }
	store.save(record):after(function(id: any)
		if type(id) ~= "string" then
			return
		end
		if t and t.routine then
			local r = t.routine
			-- (Its routine comes with it, switched off: nothing runs by the
			-- clock until the person says so.)
			ui.call("kx.automation.save", {
				buddy = id,
				title = r.title,
				prompt = r.prompt,
				kind = r.kind,
				time = r.time,
				days = r.days,
				minutes = r.minutes,
				enabled = false,
			}):after(function() end, function() end)
		end
		go_buddy(id)
	end, function(why: any)
		ds.toast.show { title = "The buddy couldn't be made", body = tostring(why), tone = "danger" }
	end)
end

local function heading(title: string, about: string?): Node
	return ui.column {
		w_full = true,
		shrink = 0,
		gap = "$space.1",
		ui.text { title, literal = true, color = "$text.primary", text_size = "$font.size.lg", weight = "$font.weight.semibold" },
		if about then ui.text { about, literal = true, wrap = true, color = "$text.secondary", text_size = "$font.size.sm" } else nil,
	}
end

local function page(children: { any }): Node
	local column: { [any]: any } = {
		w_full = true,
		max_w = 920,
		mx = "auto",
		px = "$space.6",
		py = "$space.8",
		gap = "$space.6",
	}
	for _, c in children do
		table.insert(column, c)
	end
	return ui.scroll { w_full = true, h_full = true, ui.column(column) }
end

-- ── When an automation runs, in words ───────────────────────────────────
local DAYS = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" }
local function when_words(a: store.Automation): string
	if a.kind == "interval" then
		local m = a.minutes or 60
		if m % 60 == 0 then
			return if m == 60 then "Every hour" else `Every {m // 60} hours`
		end
		return `Every {m} minutes`
	elseif a.kind == "once" then
		return if a.at then `Once, {os.date("%a %d %b %H:%M", a.at)}` else "Once"
	end
	local days = a.days or {}
	local time = a.time or "09:00"
	if #days == 0 or #days == 7 then
		return `Every day at {time}`
	end
	local set: { [number]: boolean } = {}
	for _, d in days do
		set[d] = true
	end
	if #days == 5 and set[0] and set[1] and set[2] and set[3] and set[4] then
		return `Weekdays at {time}`
	end
	local names = {}
	for d = 0, 6 do
		if set[d] then
			table.insert(names, DAYS[d + 1])
		end
	end
	return `{table.concat(names, ", ")} at {time}`
end

-- A sentence read for when it means: "every weekday at 9", "every 30
-- minutes", "daily at 5:30pm", "mondays and thursdays at 14:00". What it
-- can't read is left as it was.
local DAY_WORDS = { mon = 0, tue = 1, wed = 2, thu = 3, fri = 4, sat = 5, sun = 6 }
function M.read_when(sentence: string): { kind: string, minutes: number?, time: string?, days: { number }? }?
	local s = string.lower(sentence)
	local n = string.match(s, "every%s+(%d+)%s*min")
	if n then
		return { kind = "interval", minutes = math.max(1, tonumber(n) or 60) }
	end
	n = string.match(s, "every%s+(%d+)%s*hour")
	if n then
		return { kind = "interval", minutes = math.max(1, tonumber(n) or 1) * 60 }
	end
	if string.find(s, "every hour") or string.find(s, "hourly") then
		return { kind = "interval", minutes = 60 }
	end
	local hour, minute, half = string.match(s, "at%s+(%d%d?):?(%d?%d?)%s*([ap]?m?)")
	if not hour then
		return nil
	end
	local h, m = tonumber(hour) or 9, tonumber(minute) or 0
	if half == "pm" and h < 12 then
		h += 12
	elseif half == "am" and h == 12 then
		h = 0
	end
	if h > 23 or m > 59 then
		return nil
	end
	local days: { number } = {}
	if string.find(s, "weekday") then
		days = { 0, 1, 2, 3, 4 }
	elseif string.find(s, "weekend") then
		days = { 5, 6 }
	else
		for word, d in DAY_WORDS do
			if string.find(s, word) then
				table.insert(days, d)
			end
		end
		table.sort(days)
	end
	return { kind = "daily", time = string.format("%02d:%02d", h, m), days = days }
end

-- ── The board ───────────────────────────────────────────────────────────
local function buddy_card(b: store.Buddy): Node
	return ui.button {
		label = b.name,
		w = 200,
		shrink = 0,
		p = "$space.4",
		rounded = "$radius.xl",
		bg = "$zeron.secondary.bg",
		hover = { bg = "$zeron.secondary.hover" },
		border = 1,
		border_color = "$border",
		on_click = function()
			go_buddy(b.id)
		end,
		ui.column {
			w_full = true,
			gap = "$space.3",
			items = "start",
			Avatar { look = b.look, seed = b.id, size = 48, live = true, standing = function(): string?
				return store.standing(b)
			end },
			ui.column {
				w_full = true,
				gap = "$zeron.space.hair",
				items = "start",
				ui.text { b.name, literal = true, truncate = true, w_full = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.semibold" },
				ui.text {
					if b.role and b.role ~= "" then b.role else "No role yet",
					literal = true,
					truncate = true,
					w_full = true,
					color = "$text.secondary",
					text_size = "$font.size.sm",
				},
			},
			ui.row {
				gap = "$space.2",
				items = "center",
				ui.div { w = 6, h = 6, rounded = "$radius.full", bg = function(): string
					return standing_color(b)
				end },
				ui.text {
					function(): string
						return standing_words(b)
					end,
					literal = true,
					color = "$text.tertiary",
					text_size = "$font.size.xs",
				},
			},
		},
	}
end

local function template_card(t: templates.Template): Node
	return ui.column {
		w = 200,
		shrink = 0,
		p = "$space.4",
		gap = "$space.3",
		rounded = "$radius.xl",
		border = 1,
		border_color = "$border",
		Avatar { look = t.look, seed = t.id, size = 40 },
		ui.column {
			w_full = true,
			gap = "$zeron.space.hair",
			ui.text { t.role, literal = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.semibold" },
			ui.text { t.about, literal = true, wrap = true, color = "$text.secondary", text_size = "$font.size.sm" },
		},
		ds.Button {
			tone = "secondary",
			size = "sm",
			on_click = function()
				create(t)
			end,
			ui.text { `Add {t.name}`, literal = true },
		},
	}
end

local function run_row(r: store.Run): Node
	local b = store.by_id(r.buddy)
	local ok = r.status == "sent"
	return ListRow {
		label = r.title or "Run",
		on_click = function()
			if r.chat then
				route.open_session(r.chat)
			elseif b then
				go_buddy(b.id)
			end
		end,
		ui.row {
			w_full = true,
			px = "$space.3",
			py = "$space.2",
			gap = "$space.3",
			items = "center",
			ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = if ok then "$success" elseif r.status == "skipped" then "$text.tertiary" else "$danger" },
			ui.column {
				grow = 1,
				min_w = 0,
				gap = "$zeron.space.hair",
				ui.text {
					`{if b then b.name else "A buddy that's gone"} · {r.title or "Run"}`,
					literal = true,
					truncate = true,
					color = "$text.primary",
					text_size = "$font.size.md",
				},
				ui.text {
					if r.note and r.note ~= "" then r.note elseif ok then "Sent to its chat." else r.status,
					literal = true,
					truncate = true,
					color = "$text.tertiary",
					text_size = "$font.size.sm",
				},
			},
			ui.text {
				status.ago(math.max(0, os.time() - r.at)),
				literal = true,
				shrink = 0,
				color = "$text.tertiary",
				text_size = "$font.size.sm",
				tabular = true,
			},
		},
	}
end

local function runs_list(of: () -> { store.Run }, none: string): Node
	local shown = ui.computed(of)
	return ui.column {
		w_full = true,
		gap = 0,
		ui.show(function(): boolean
			return #shown:get() == 0
		end, ui.text { none, literal = true, wrap = true, color = "$text.tertiary", text_size = "$font.size.sm" }),
		ui.each(shown, function(r: any): Node
			return run_row(r)
		end, {
			key = function(r: any): string
				return `{r.run}:{r.status}`
			end,
		}),
	}
end

function M.board(): Node
	local starters: { [any]: any } = { w_full = true, gap = "$space.3", flex_wrap = true, items = "stretch" }
	for _, t in templates.list do
		table.insert(starters, template_card(t))
	end
	return page {
		ui.row {
			w_full = true,
			gap = "$space.3",
			items = "start",
			ui.column {
				grow = 1,
				min_w = 0,
				gap = "$space.1",
				ui.text { "Buddies", literal = true, color = "$text.primary", text_size = "$font.size.xl", weight = "$font.weight.semibold" },
				ui.text {
					"Bots with a job of their own. Each has standing instructions, runs on any agent you're signed into, and can be sent work by the clock.",
					literal = true,
					wrap = true,
					color = "$text.secondary",
					text_size = "$font.size.md",
				},
			},
			ds.Button {
				size = "sm",
				on_click = function()
					create(nil)
				end,
				Icon { name = "plus", size = 14 },
				ui.text { "New buddy", literal = true },
			},
		},
		ui.column {
			w_full = true,
			gap = "$space.3",
			ui.show(function(): boolean
				return #(store.list:get() or {}) == 0
			end, ui.text { "No buddies yet. Start with one of the four below, or make your own.", literal = true, color = "$text.tertiary", text_size = "$font.size.sm" }),
			ui.row {
				w_full = true,
				gap = "$space.3",
				flex_wrap = true,
				ui.each(store.list, function(b: any): Node
					return buddy_card(b)
				end, {
					key = function(b: any): string
						local l = b.look or {}
						return `{b.id}:{b.name}:{b.role or ""}:{l.shape or ""}{l.color or ""}{l.face or ""}:{b.chat or ""}`
					end,
				}),
			},
		},
		ui.column { w_full = true, gap = "$space.3", heading("Start from a template", "Each comes with its job written out; rewrite it as you like."), ui.row(starters) },
		ui.column {
			w_full = true,
			gap = "$space.3",
			heading("Last runs", "What the clock sent to your buddies."),
			runs_list(function(): { store.Run }
				local out: { store.Run } = {}
				for i, r in store.runs:get() or {} do
					if i > 8 then
						break
					end
					table.insert(out, r)
				end
				return out
			end, "Nothing has run yet. A buddy's Automations tab is where its routines are set."),
		},
	}
end

-- ── A buddy's profile ───────────────────────────────────────────────────
local function pick_row(title: string, options: { string }, value: State<string>, look_of: (string) -> Avatar.Look, seed: string): Node
	local row: { [any]: any } = { gap = "$space.2", flex_wrap = true, items = "center" }
	for _, id in options do
		table.insert(row, ui.button {
			label = `{title}: {id}`,
			w = 40,
			h = 40,
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = "$radius.lg",
			bg = function(): string
				return if value:get() == id then "$zeron.selected.bg" else "#00000000"
			end,
			hover = { bg = "$hover" },
			border = 1,
			border_color = function(): string
				return if value:get() == id then "$zeron.selected.ring" else "$border.subtle"
			end,
			on_click = function()
				value:set(id)
			end,
			Avatar { look = function(): Avatar.Look?
				return look_of(id)
			end, seed = seed, size = 26 },
		})
	end
	return ui.row(row)
end

-- One step of what it runs on: an agent, and which of its models.
local Step = ui.component(function(props: { step: store.Step, index: number, steps: State<{ store.Step }> }): Node
	local harness = ui.use_state(props.step.harness)
	local model = ui.use_state(props.step.model or "")
	local i = props.index
	ui.effect(function()
		local h, m = harness:get(), model:get()
		local list = props.steps:peek()
		local was = list[i]
		if was and (was.harness ~= h or (was.model or "") ~= m) then
			local next = table.clone(list)
			-- (Another agent: its models aren't the last one's.)
			next[i] = { harness = h, model = if was.harness ~= h or m == "" then nil else m }
			props.steps:set(next)
		end
	end)
	local agent_options = {}
	local this: store.Agent? = nil
	for _, a in store.agents() do
		table.insert(agent_options, { value = a.harness, label = if a.ok then a.label else `{a.label} ({a.why or "not available"})` })
		if a.harness == props.step.harness then
			this = a
		end
	end
	local model_options: { any } = { { value = "", label = "Its default model" } }
	for _, m in store.models(props.step.harness) do
		table.insert(model_options, m)
	end
	return ui.row {
		w_full = true,
		px = "$space.3",
		py = "$space.2",
		gap = "$space.2",
		items = "center",
		ui.text { `{i}.`, literal = true, w = 18, shrink = 0, color = "$text.tertiary", text_size = "$font.size.sm", tabular = true },
		ui.div { grow = 1, min_w = 0, ds.Select { value = harness, options = agent_options } },
		ui.div { grow = 1, min_w = 0, ds.Select { value = model, options = model_options } },
		ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = if this and this.ok then "$success" else "$danger", tooltip = if this and this.ok then "Usable now" else (if this then this.why else "Not installed here") or "Not available" },
		ds.IconButton {
			icon = "close" :: any,
			label = "Remove this step",
			size = "sm",
			tone = "quiet",
			on_click = function()
				local next = table.clone(props.steps:peek())
				table.remove(next, i)
				props.steps:set(next)
			end,
		},
	}
end)

local Profile = ui.component(function(props: { buddy: store.Buddy }): Node
	local b = props.buddy
	local given = Avatar.resolve(b.look, b.id)
	local name = ui.use_state(b.name)
	local role = ui.use_state(b.role or "")
	local job = ui.use_state(b.instructions or "")
	local folder = ui.use_state(b.folder or "")
	local may = ui.use_state(b.permissions or "draft")
	local shape = ui.use_state(given.shape)
	local color = ui.use_state(given.color)
	local face = ui.use_state(given.face)
	local steps = ui.use_state(table.clone(b.models or {}) :: { store.Step })
	local saving = ui.use_state(false)

	local function look_now(): Avatar.Look
		return { shape = shape:get(), color = color:get(), face = face:get() }
	end
	local function save()
		saving:set(true)
		store.save({
			id = b.id,
			name = name:peek(),
			role = role:peek(),
			instructions = job:peek(),
			folder = folder:peek(),
			permissions = may:peek(),
			look = { shape = shape:peek(), color = color:peek(), face = face:peek() },
			models = steps:peek(),
		}):after(function()
			saving:set(false)
			ds.toast.show { title = "Saved", tone = "success" }
		end, function(why: any)
			saving:set(false)
			ds.toast.show { title = "It couldn't be saved", body = tostring(why), tone = "danger" }
		end)
	end
	local color_ids = {}
	for _, c in Avatar.COLORS do
		table.insert(color_ids, c.id)
	end
	return ui.column {
		w_full = true,
		gap = "$space.6",
		ds.Settings.Section {
			title = "Who it is",
			ds.Settings.Group {
				ds.Settings.Row { label = "Name", stacked = true, control = ds.TextField { value = name, placeholder = "Its name", w_full = true } },
				ds.Settings.Row { label = "Role", description = "A few words: what it is to you.", stacked = true, control = ds.TextField { value = role, placeholder = "Researcher", w_full = true } },
				ds.Settings.Row {
					label = "Its job",
					description = "Its standing instructions. They're written into its folder as the files agents read by themselves, so they hold on every turn.",
					stacked = true,
					control = ds.TextArea { w_full = true, value = job, rows = 5, max_rows = 16, placeholder = "What should it look after, and how should it report back?" },
				},
			},
		},
		ds.Settings.Section {
			title = "Its face",
			ds.Settings.Group {
				ds.Settings.Row {
					label = "Shape",
					stacked = true,
					control = pick_row("Shape", Avatar.SHAPES, shape, function(id: string): Avatar.Look
						return { shape = id, color = color:get(), face = face:get() }
					end, b.id),
				},
				ds.Settings.Row {
					label = "Colour",
					stacked = true,
					control = pick_row("Colour", color_ids, color, function(id: string): Avatar.Look
						return { shape = shape:get(), color = id, face = face:get() }
					end, b.id),
				},
				ds.Settings.Row {
					label = "Expression",
					stacked = true,
					control = pick_row("Expression", Avatar.FACES, face, function(id: string): Avatar.Look
						return { shape = shape:get(), color = color:get(), face = id }
					end, b.id),
				},
			},
		},
		ds.Settings.Section {
			title = "What it runs on",
			description = "In order. A run takes the first that's signed in and under its limit, and says so in the chat when that isn't the first.",
			ds.Settings.Group {
				ui.column {
					w_full = true,
					ui.show(function(): boolean
						return #steps:get() == 0
					end, ui.text {
						"Nothing chosen: it runs on whichever agent is usable.",
						literal = true,
						px = "$space.3",
						py = "$space.3",
						color = "$text.tertiary",
						text_size = "$font.size.sm",
					}),
					-- (Made again when the agents on offer change: the list of them can
					-- arrive after this page shows, and a step built before it would
					-- name none.)
					ui.presence(store.agents_key, function(): Node
						return ui.column {
							w_full = true,
							ui.each(steps, function(step: any, index: number): Node
								return Step { step = step, index = index, steps = steps }
							end, {
								key = function(step: any): string
									-- (Made again when its agent changes, or the list's length does.)
									return `{step.harness}:{#steps:peek()}`
								end,
							}),
						}
					end, { w_full = true }),
				},
				ui.row {
					w_full = true,
					px = "$space.3",
					py = "$space.2",
					ds.Button {
						tone = "ghost",
						size = "sm",
						on_click = function()
							local offered = store.agents()
							local taken: { [string]: boolean } = {}
							for _, s in steps:peek() do
								taken[s.harness] = true
							end
							for _, a in offered do
								if not taken[a.harness] then
									local next = table.clone(steps:peek())
									table.insert(next, { harness = a.harness })
									steps:set(next)
									return
								end
							end
							ds.toast.show { title = "Every agent you have is in its order already", tone = "info" }
						end,
						Icon { name = "plus", size = 14 },
						ui.text {
							function(): string
								return if #steps:get() == 0 then "Choose an agent" else "Add a fallback"
							end,
							literal = true,
						},
					},
				},
			},
		},
		ds.Settings.Section {
			title = "Where and what",
			ds.Settings.Group {
				ds.Settings.Row {
					label = "Folder it looks after",
					description = function(): string
						return `Optional: a project's folder. Its own notes and drafts live in {b.home or "its own folder"}.`
					end,
					stacked = true,
					control = ds.TextField { value = folder, placeholder = "C:\\path\\to\\a\\project", w_full = true },
				},
				ds.Settings.Row {
					label = "What it may do without asking",
					stacked = true,
					control = ds.Segmented {
						value = may,
						fill = true,
						options = {
							{ value = "draft", label = "Read and draft" },
							{ value = "folder", label = "Anything in its folders" },
							{ value = "ask", label = "Ask for everything" },
						},
					},
				},
			},
		},
		ui.row {
			w_full = true,
			gap = "$space.3",
			items = "center",
			ds.Button {
				disabled = function(): boolean
					return saving:get()
				end,
				on_click = save,
				ui.text { "Save", literal = true },
			},
			Avatar { look = look_now, seed = b.id, size = 32, live = true },
			ui.text { "How it will look.", literal = true, color = "$text.tertiary", text_size = "$font.size.sm" },
		},
	}
end)

-- ── A buddy's computer ──────────────────────────────────────────────────
-- A sandbox of its own: on this PC (through cua) or in the cloud (on Boat,
-- with the person's key). Made here, started, stopped, removed, and its
-- desktop opened in the panel beside its chat.
local IMAGES = {
	{ value = "omarchy", label = "Omarchy (Arch Linux desktop)" },
	{ value = "linux", label = "Ubuntu, minimal" },
	{ value = "linux-full", label = "Ubuntu, with developer tools" },
	{ value = "windows", label = "Windows Server 2022" },
}
local STANDING: { [string]: string } = {
	ready = "Running",
	running = "Running",
	idle = "Running, idle",
	stopped = "Stopped",
	suspended = "Stopped",
	archived = "Stopped",
	archiving = "Stopping",
	creating = "Being made",
	starting = "Starting",
	init = "Being made",
	provisioning = "Being made",
	provisioned = "Starting",
	cloning = "Being made",
	error = "Failed",
	cancelled = "Cancelled",
}
local UP: { [string]: boolean } = { ready = true, running = true, idle = true }

local Computer = ui.component(function(props: { buddy: store.Buddy }): Node
	local b = props.buddy
	local where = ui.use_state(if b.computer and b.computer.kind == "boat" then "boat" else "local")
	local image = ui.use_state("omarchy")
	local busy = ui.use_state(nil :: string?)
	local key = ui.use_state("")
	local sure = ui.use_state(false)
	store.read_computers()
	-- (Boat is asked once the cloud is what's chosen and there's a key to ask with.)
	ui.effect(function()
		if where:get() == "boat" and store.keys:get().boat and not store.boats_read:peek() then
			store.read_boats()
		end
	end)
	local function cloud(): boolean
		return where:get() == "boat"
	end
	-- The one it has, of the kind chosen: what to call it, how it stands.
	local function has(): (string?, string?)
		if cloud() then
			local c = store.boat_of(b)
			return c and c.id, c and c.state
		end
		local c = store.computer_of(b)
		return c and c.name, c and c.status
	end
	local function read()
		if where:peek() == "boat" then
			store.read_boats()
		else
			store.read_computers()
		end
	end
	local function act(what: string, verb: string, said: string, then_: (() -> ())?)
		local target = has()
		if not target then
			return
		end
		busy:set(what)
		ui.call(`{if where:peek() == "boat" then "kx.boat." else "kx.computer."}{verb}`, target):after(function()
			busy:set(nil)
			read()
			if then_ then
				then_()
			end
			ds.toast.show { title = said, tone = "success" }
		end, function(why: any)
			busy:set(nil)
			read()
			ds.toast.show { title = "That didn't work", body = tostring(why), tone = "danger" }
		end)
	end
	local function state(): string
		if busy:get() then
			return "busy"
		elseif cloud() then
			if not store.keys:get().boat then
				return "key"
			elseif not store.boats_read:get() then
				return "reading"
			elseif store.boats_error:get() then
				return "unreachable"
			end
		else
			if not store.computer_ready:get() then
				return "none"
			elseif not store.computers_read:get() then
				return "reading"
			end
		end
		local target, standing = has()
		if not target then
			return if cloud() then "make-boat" else "make"
		end
		return if UP[standing or ""] then "running" else "stopped"
	end
	local function made(ok: boolean, why: string?)
		busy:set(nil)
		if ok then
			ds.toast.show { title = `{b.name} has a computer`, tone = "success" }
		else
			ds.toast.show { title = "Its computer couldn't be made", body = why, tone = "danger" }
		end
	end
	local function forget_key()
		pcall(ui.invoke, "kx.key.remove", "boat")
		store.boats:set({})
		store.boats_read:set(false)
		ds.toast.show { title = "The Boat key is forgotten", tone = "success" }
	end
	return ds.Settings.Section {
		title = "Its computer",
		description = "A desktop of its own to work in, which you can watch in the panel beside its chat. One on this PC is free and stops when the PC sleeps. One in the cloud keeps going with the PC off, and is billed by Boat while it runs.",
		ds.Settings.Group {
			ds.Settings.Row {
				label = "Where",
				stacked = true,
				control = ds.Segmented {
					value = where,
					fill = true,
					options = { { value = "local", label = "This PC" }, { value = "boat", label = "Cloud (Boat)" } },
				},
			},
			ui.presence(state, function(which: string): Node
				if which == "none" then
					return ds.Settings.Row { label = "cua isn't installed", description = "A computer on this PC is a cua sandbox. Install cua (cua.ai), then open this page again." }
				elseif which == "busy" then
					return ds.Settings.Row {
						label = function(): string
							return busy:get() or "Working"
						end,
						description = "This can take a few minutes: the machine is being made and started.",
					}
				elseif which == "reading" then
					return ds.Settings.Row { label = "Looking for its computer" }
				elseif which == "unreachable" then
					return ui.column {
						w_full = true,
						ds.Settings.Row {
							label = "Boat didn't answer",
							description = function(): string
								return store.boats_error:get() or ""
							end,
						},
						ui.row {
							w_full = true,
							px = "$space.3",
							py = "$space.2",
							gap = "$space.2",
							ds.Button {
								tone = "secondary",
								size = "sm",
								on_click = function()
									store.boats_read:set(false)
									store.read_boats()
								end,
								ui.text { "Try again", literal = true },
							},
							ds.Button { tone = "ghost", size = "sm", on_click = forget_key, ui.text { "Forget the key", literal = true } },
						},
					}
				elseif which == "key" then
					return ui.column {
						w_full = true,
						ds.Settings.Row {
							label = "Your Boat key",
							description = "Boat (boat.dev) rents Linux desktops for agents; it's a paid service of its own. Make an API key in your Boat account and paste it here. It's sealed on this PC for your Windows account and never shown again.",
							stacked = true,
							control = ds.TextField { value = key, password = true, placeholder = "boat_…", w_full = true },
						},
						ui.row {
							w_full = true,
							px = "$space.3",
							py = "$space.2",
							ds.Button {
								tone = "secondary",
								size = "sm",
								on_click = function()
									local value = key:peek()
									key:set("")
									busy:set("Checking the key with Boat")
									ui.call("kx.key.set", { name = "boat", value = value }):after(function()
										ui.call("kx.boat.check"):after(function()
											busy:set(nil)
											store.read_boats()
											ds.toast.show { title = "Boat took the key", tone = "success" }
										end, function(why: any)
											-- (A key Boat won't take isn't kept.)
											pcall(ui.invoke, "kx.key.remove", "boat")
											busy:set(nil)
											ds.toast.show { title = "The key wasn't kept", body = tostring(why), tone = "danger" }
										end)
									end, function(why: any)
										busy:set(nil)
										ds.toast.show { title = "The key wasn't kept", body = tostring(why), tone = "danger" }
									end)
								end,
								ui.text { "Save key", literal = true },
							},
						},
					}
				elseif which == "make" then
					return ui.column {
						w_full = true,
						ds.Settings.Row { label = "It has none yet", description = "Choose what it runs. An image not yet on this PC is downloaded first (a few gigabytes).", control = ui.div { w = 260, ds.Select { value = image, options = IMAGES } } },
						ui.row {
							w_full = true,
							px = "$space.3",
							py = "$space.2",
							ds.Button {
								tone = "secondary",
								size = "sm",
								on_click = function()
									busy:set("Making its computer")
									store.make_computer(b, image:peek(), made)
								end,
								ui.text { "Make its computer", literal = true },
							},
						},
					}
				elseif which == "make-boat" then
					return ui.column {
						w_full = true,
						ds.Settings.Row { label = "It has none yet", description = "A Linux desktop on Boat, billed to your Boat account while it runs. It stops itself an hour after it starts, so it can't run up a bill overnight; start it again when it's needed." },
						ui.row {
							w_full = true,
							px = "$space.3",
							py = "$space.2",
							gap = "$space.2",
							ds.Button {
								tone = "secondary",
								size = "sm",
								on_click = function()
									busy:set("Making its computer on Boat")
									store.make_boat(b, made)
								end,
								ui.text { "Make its computer", literal = true },
							},
							ds.Button { tone = "ghost", size = "sm", on_click = forget_key, ui.text { "Forget the key", literal = true } },
						},
					}
				end
				local running = which == "running"
				return ui.column {
					w_full = true,
					ds.Settings.Row {
						label = store.computer_name(b),
						description = function(): string
							local _, standing = has()
							local word = STANDING[standing or ""] or standing or "Unknown"
							return if cloud() then `{word} · on Boat` else `{word} · on this PC`
						end,
					},
					ui.row {
						w_full = true,
						px = "$space.3",
						py = "$space.2",
						gap = "$space.2",
						flex_wrap = true,
						ds.Button {
							size = "sm",
							disabled = not running,
							on_click = function()
								store.open_desktop(b, function(ok: boolean, why: string?)
									if not ok then
										ds.toast.show { title = "Its desktop couldn't be shown", body = why, tone = "danger" }
									end
								end)
							end,
							ui.text { "Open desktop", literal = true },
						},
						ds.Button {
							tone = "secondary",
							size = "sm",
							on_click = function()
								if running then
									act("Stopping it", "stop", "Stopped")
								else
									act("Starting it", "start", "Started")
								end
							end,
							ui.text { if running then "Stop" else "Start", literal = true },
						},
						ds.Button {
							tone = "ghost",
							size = "sm",
							on_click = function()
								-- (Asked twice: what's on it goes with it.)
								if not sure:peek() then
									sure:set(true)
									return
								end
								sure:set(false)
								act("Removing it", "remove", "Its computer is removed", function()
									store.forget_computer(b)
								end)
							end,
							ui.text {
								function(): string
									return if sure:get() then "Remove it and everything on it?" else "Remove"
								end,
								literal = true,
							},
						},
					},
				}
			end, { w_full = true }),
		},
	}
end)

-- ── A buddy's automations ───────────────────────────────────────────────
local function mine(b: store.Buddy): { store.Automation }
	local out: { store.Automation } = {}
	for _, a in store.automations:get() or {} do
		if a.buddy == b.id then
			table.insert(out, a)
		end
	end
	return out
end

local AutomationRow = ui.component(function(props: { a: store.Automation }): Node
	local a = props.a
	local on = ui.use_state(a.enabled ~= false)
	ui.effect(function()
		local want = on:get()
		if want ~= (a.enabled ~= false) then
			local next: { [string]: any } = table.clone(a :: any)
			next.enabled = want
			ui.call("kx.automation.save", next):after(function() end, function() end)
		end
	end)
	return ui.row {
		w_full = true,
		px = "$space.3",
		py = "$space.3",
		gap = "$space.3",
		items = "center",
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$zeron.space.hair",
			ui.text { a.title or "Routine", literal = true, truncate = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
			ui.text {
				`{when_words(a)}{if a.enabled ~= false and a.next then " · next " .. tostring(os.date("%a %H:%M", a.next)) else " · off"}`,
				literal = true,
				truncate = true,
				color = "$text.tertiary",
				text_size = "$font.size.sm",
			},
			ui.text { a.prompt, literal = true, truncate = true, color = "$text.secondary", text_size = "$font.size.sm" },
		},
		ds.Button {
			tone = "ghost",
			size = "sm",
			on_click = function()
				pcall(ui.invoke, "kx.automation.run_now", a.id)
				ds.toast.show { title = "Sending it now", tone = "info" }
			end,
			ui.text { "Run now", literal = true },
		},
		ds.Switch { value = on, label = `{a.title or "Routine"} on` },
		ds.IconButton {
			icon = "trash-bin-minimalistic" :: any,
			label = "Delete this routine",
			size = "sm",
			tone = "quiet",
			on_click = function()
				pcall(ui.invoke, "kx.automation.remove", a.id)
			end,
		},
	}
end)

local Automations = ui.component(function(props: { buddy: store.Buddy }): Node
	local b = props.buddy
	local sentence = ui.use_state("")
	local prompt = ui.use_state("")
	local title = ui.use_state("")
	local read = ui.computed(function(): any
		return M.read_when(sentence:get())
	end)
	local list = ui.computed(function(): { store.Automation }
		return mine(b)
	end)
	local function add()
		local w = read:peek()
		if not w then
			ds.toast.show { title = "Say when", body = "For example: every weekday at 9, or every 30 minutes.", tone = "warning" }
			return
		end
		if string.match(prompt:peek(), "^%s*$") then
			ds.toast.show { title = "Say what to send it", tone = "warning" }
			return
		end
		local name = title:peek()
		ui.call("kx.automation.save", {
			buddy = b.id,
			title = if string.match(name, "^%s*$") then string.sub(prompt:peek(), 1, 48) else name,
			prompt = prompt:peek(),
			kind = w.kind,
			minutes = w.minutes,
			time = w.time,
			days = w.days,
			enabled = true,
		}):after(function()
			sentence:set("")
			prompt:set("")
			title:set("")
		end, function(why: any)
			ds.toast.show { title = "It couldn't be added", body = tostring(why), tone = "danger" }
		end)
	end
	return ui.column {
		w_full = true,
		gap = "$space.6",
		ds.Settings.Section {
			title = "Routines",
			description = "Each sends its message to this buddy's chat by the clock, while Keplorayx is open. One that comes due while it's closed isn't replayed.",
			ds.Settings.Group {
				ui.column {
					w_full = true,
					ui.show(function(): boolean
						return #list:get() == 0
					end, ui.text { "No routines yet.", literal = true, px = "$space.3", py = "$space.3", color = "$text.tertiary", text_size = "$font.size.sm" }),
					ui.each(list, function(a: any): Node
						return AutomationRow { a = a }
					end, {
						key = function(a: any): string
							return `{a.id}:{a.title or ""}:{a.kind}:{a.minutes or 0}:{a.time or ""}:{tostring(a.enabled)}:{a.next or 0}`
						end,
					}),
				},
			},
		},
		ds.Settings.Section {
			title = "Add a routine",
			ds.Settings.Group {
				ds.Settings.Row {
					label = "When",
					description = function(): string
						local w = read:get()
						if string.match(sentence:get(), "^%s*$") then
							return "Say it in words: every weekday at 9, every 30 minutes, mondays and thursdays at 14:00."
						end
						return if w then `Read as: {when_words(w :: any)}.` else "That isn't a time I can read yet."
					end,
					stacked = true,
					control = ds.TextField { value = sentence, placeholder = "every weekday at 9", w_full = true },
				},
				ds.Settings.Row {
					label = "What to send it",
					stacked = true,
					control = ds.TextArea { w_full = true, value = prompt, rows = 3, max_rows = 8, placeholder = "Summarise my open pull requests." },
				},
				ds.Settings.Row { label = "Name", description = "Optional.", stacked = true, control = ds.TextField { value = title, placeholder = "Morning summary", w_full = true } },
				ui.row { w_full = true, px = "$space.3", py = "$space.2", ds.Button { size = "sm", on_click = add, ui.text { "Add routine", literal = true } } },
			},
		},
	}
end)

-- ── The buddy's page ────────────────────────────────────────────────────
local tab = ui.state("kx_buddy_tab", "profile")
local deleting = ui.state("kx_buddy_deleting", false)

local function buddy_page(id: string): Node
	local b = store.by_id(id)
	if not b then
		return page { ds.EmptyState { icon = "users-group-rounded", size = "sm", title = "That buddy is gone", body = "It was removed. The others are on the board." } }
	end
	local buddy: store.Buddy = b
	return page {
		ui.row {
			w_full = true,
			gap = "$space.4",
			items = "center",
			ds.IconButton { icon = "alt-arrow-left" :: any, label = "All buddies", size = "sm", tone = "quiet", on_click = go_board },
			Avatar { look = buddy.look, seed = buddy.id, size = 56, live = true, standing = function(): string?
				return store.standing(buddy)
			end },
			ui.column {
				grow = 1,
				min_w = 0,
				gap = "$zeron.space.hair",
				ui.text { buddy.name, literal = true, truncate = true, color = "$text.primary", text_size = "$font.size.xl", weight = "$font.weight.semibold" },
				ui.row {
					gap = "$space.2",
					items = "center",
					ui.text { if buddy.role and buddy.role ~= "" then buddy.role else "No role yet", literal = true, color = "$text.secondary", text_size = "$font.size.md" },
					ui.div { w = 6, h = 6, rounded = "$radius.full", bg = function(): string
						return standing_color(buddy)
					end },
					ui.text {
						function(): string
							return standing_words(buddy)
						end,
						literal = true,
						color = "$text.tertiary",
						text_size = "$font.size.sm",
					},
				},
			},
			ui.show(function(): boolean
				if buddy.computer and buddy.computer.kind == "boat" then
					return store.boat_of(buddy) ~= nil
				end
				return store.computer_of(buddy) ~= nil
			end, ds.Button {
				tone = "secondary",
				size = "sm",
				on_click = function()
					store.open_desktop(buddy, function(ok: boolean, why: string?)
						if not ok then
							ds.toast.show { title = "Its desktop couldn't be shown", body = why, tone = "danger" }
						end
					end)
				end,
				ui.text { "Desktop", literal = true },
			}),
			ds.Button {
				size = "sm",
				on_click = function()
					store.open(buddy)
				end,
				Icon { name = "chat-round-line" :: any, size = 14 },
				ui.text { "Open chat", literal = true },
			},
			ds.IconButton {
				icon = "trash-bin-minimalistic" :: any,
				label = "Remove this buddy",
				size = "sm",
				tone = "quiet",
				on_click = function()
					deleting:set(true)
				end,
			},
		},
		ds.Tabs {
			value = tab,
			options = {
				{ value = "profile", label = "Profile" },
				{ value = "automations", label = "Automations", count = function(): number
					return #mine(buddy)
				end },
				{ value = "activity", label = "Activity" },
			},
		},
		ui.presence(function(): string
			return tab:get()
		end, function(which: string): Node
			if which == "automations" then
				return Automations { buddy = buddy }
			elseif which == "activity" then
				return runs_list(function(): { store.Run }
					local out: { store.Run } = {}
					for _, r in store.runs:get() or {} do
						if r.buddy == buddy.id then
							table.insert(out, r)
						end
					end
					return out
				end, "Nothing has run for it yet.")
			end
			return ui.column { w_full = true, gap = "$space.6", Profile { buddy = buddy }, Computer { buddy = buddy } }
		end, { w_full = true }),
		ds.Dialog {
			open = deleting,
			title = `Remove {buddy.name}?`,
			body = "Its routines go with it and its chat is archived. Its folder, with what it wrote, stays on disk.",
			tone = "danger",
			confirm = {
				label = "Remove",
				on_click = function()
					deleting:set(false)
					if buddy.chat then
						sessions.archive(buddy.chat)
					end
					store.remove(buddy.id)
					go_board()
				end,
			},
		},
	}
end

function M.buddy(): Node
	return ui.column {
		w_full = true,
		h_full = true,
		class = "kx-buddy",
		ui.presence(function(): string
			-- (Made again for another buddy, or once this one is saved under
			-- another name or look: its head follows.)
			local b = store.by_id(store.selected:get())
			if not b then
				return ""
			end
			local l = b.look or {}
			return `{b.id}|{b.name}|{b.role or ""}|{l.shape or ""}{l.color or ""}{l.face or ""}`
		end, function(key: string): Node
			local id = string.match(key, "^([^|]*)") or ""
			return buddy_page(id)
		end, { w_full = true, h_full = true }),
	}
end

return M
