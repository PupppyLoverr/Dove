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
-- A buddy set up where Grok Bot sets one up: in its chat, its drawer on
-- its settings. (Just made, it may not be listed yet: a moment later.)
local function go_buddy(id: string)
	store.selected:set(id)
	store.page_tab:set("profile")
	ui.after(100, function()
		local b = store.by_id(id)
		if b then
			store.show(b, "settings")
		end
	end)
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
	return ui.scroll { w_full = true, h_full = true, on_pointer_move = Avatar.track, on_hover = function(inside: boolean) if not inside then Avatar.untrack() end end, ui.column(column) }
end

-- A run of related settings: a quiet title over them, and no lines.
local function Section(props: { [any]: any }): Node
	local col: { [any]: any } = { w_full = true, gap = "$space.3" }
	if props.title then
		table.insert(col, ui.column {
			gap = "$zeron.space.hair",
			ui.text { props.title, literal = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.semibold" },
			if props.description then ui.text { props.description, literal = true, color = "$text.tertiary", text_size = "$font.size.sm" } else nil,
		})
	end
	for _, c in ipairs(props) do
		table.insert(col, c)
	end
	return ui.column(col)
end

-- Its rows on one soft surface, spaced, with nothing ruled between them.
local function Group(props: { [any]: any }): Node
	local col: { [any]: any } = { w_full = true, rounded = "$radius.xl", bg = "$zeron.settings.card", py = "$space.1" }
	for _, c in ipairs(props) do
		table.insert(col, c)
	end
	return ui.column(col)
end

-- ── When an automation runs, in words ───────────────────────────────────
local DAYS = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" }
local function when_words(a: store.Automation): string
	if a.kind == "webhook" then
		return "By webhook"
	elseif a.kind == "interval" then
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

-- A sentence read for when it means: "every weekday at 9", "weekdays 9am",
-- "every 30m", "every 2 hours", "in 10 minutes", "daily at 5:30pm".
local DAY_WORDS = { mon = 0, tue = 1, wed = 2, thu = 3, fri = 4, sat = 5, sun = 6 }
function M.read_when(sentence: string): { kind: string, minutes: number?, time: string?, days: { number }?, at: number? }?
	local s = string.lower(sentence)
	local once_m = string.match(s, "in%s+(%d+)%s*m")
	if once_m then
		return { kind = "once", at = os.time() + math.max(1, tonumber(once_m) or 10) * 60 }
	end
	local once_h = string.match(s, "in%s+(%d+)%s*h")
	if once_h then
		return { kind = "once", at = os.time() + math.max(1, tonumber(once_h) or 1) * 3600 }
	end
	local n = string.match(s, "every%s+(%d+)%s*m")
	if n then
		return { kind = "interval", minutes = math.max(1, tonumber(n) or 60) }
	end
	n = string.match(s, "every%s+(%d+)%s*h")
	if n then
		return { kind = "interval", minutes = math.max(1, tonumber(n) or 1) * 60 }
	end
	if string.find(s, "every hour") or string.find(s, "hourly") then
		return { kind = "interval", minutes = 60 }
	end
	local hour, minute, half = string.match(s, "at%s+(%d%d?):?(%d?%d?)%s*([ap]?m?)")
	if not hour then
		hour, minute, half = string.match(s, "(%d%d?):?(%d?%d?)%s*([ap]m)")
	end
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
local function starter(t: templates.Template): Node
	return ui.button {
		label = `Add {t.name}, {t.role}`,
		h = 60,
		pl = "$space.2",
		pr = "$space.3",
		gap = "$space.3",
		items = "center",
		rounded = "$radius.lg",
		bg = "#00000000",
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		on_click = function()
			create(t)
		end,
		Avatar { look = t.look, seed = t.id, size = 40, live = true },
		ui.column {
			items = "start",
			gap = 0,
			ui.text { t.name, literal = true, color = "$text.primary", text_size = "$font.size.sm", weight = "$font.weight.medium" },
			ui.text { t.role, literal = true, color = "$text.tertiary", text_size = "$font.size.xs" },
		},
		Icon { name = "plus", size = 14, color = "$text.tertiary" },
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
					(function(s: string): string
						return string.upper(string.sub(s, 1, 1)) .. string.sub(s, 2)
					end)(if r.note and r.note ~= "" then r.note elseif ok then "Sent to its chat." else r.status),
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
			w_full = true,
			key = function(r: any): string
				return `{r.run}:{r.status}`
			end,
		}),
	}
end

-- Bringing bots across from Grok Bot: where from (Grok Bot on this PC, or a
-- backup folder), then which of its bots, all ticked; each brought with its
-- history (src/kx/grokbot.rs). One here already by name gets what's new
-- added to it.
local import_open = ui.state("kx_grokbot_import_open", false)
local import_from = ui.state("kx_grokbot_import_from", nil :: string?)

local GrokImport = ui.component(function(): Node
	local bots = ui.use_state(nil :: { any }?)
	local ticked = ui.use_state({} :: { [string]: boolean })
	local busy = ui.use_state(false)
	local failed = ui.use_state(nil :: string?)
	ui.effect(function()
		local from = import_from:get()
		if not from then
			return
		end
		bots:set(nil)
		failed:set(nil)
		ui.call("kx.grokbot.preview", { path = from }):after(function(list: any)
			local all = if type(list) == "table" then list else {}
			local on: { [string]: boolean } = {}
			for _, b in all do
				on[b.name] = true
			end
			ticked:set(on)
			bots:set(all)
		end, function(why: any)
			failed:set(tostring(why))
			bots:set({})
		end)
	end)
	local function chosen(): { string }
		local out = {}
		for _, b in bots:get() or {} do
			if ticked:get()[b.name] then
				table.insert(out, b.name)
			end
		end
		return out
	end
	local function run()
		local from = import_from:peek()
		local names = chosen()
		if not from or #names == 0 then
			return
		end
		busy:set(true)
		local everyone = #names == #(bots:peek() or {})
		ui.call("kx.grokbot.import", { path = from, names = if everyone then nil else names }):after(function(res: any)
			busy:set(false)
			import_open:set(false)
			local made, again = #(res.made or {}), res.again or 0
			local body = `{res.files or 0} files copied`
			if again > 0 then
				body ..= `, {again} added to ones already here`
			end
			if (res.skipped or 0) > 0 then
				body ..= `, {res.skipped} left out (too big)`
			end
			ds.toast.show { title = `Brought across {made + again} from Grok Bot`, body = body, tone = "success" }
		end, function(why: any)
			busy:set(false)
			ds.toast.show { title = "The import didn't finish", body = tostring(why), tone = "danger" }
		end)
	end
	return ds.Dialog {
		open = import_open,
		title = "Import from Grok Bot",
		size = "md",
		confirm = {
			label = function(): string
				local n = #chosen()
				return if n == 1 then "Import 1 bot" else `Import {n} bots`
			end,
			keep_open = true,
			busy = function(): boolean
				return busy:get()
			end,
			on_click = run,
		},
		ui.column {
			w_full = true,
			gap = "$space.2",
			ui.show(function(): boolean
				return bots:get() == nil
			end, ui.column { w_full = true, ds.SkeletonRow { lead = 32, title = 120, line = 80 }, ds.SkeletonRow { lead = 32, title = 150, line = 60 }, ds.SkeletonRow { lead = 32, title = 100, line = 90 } }),
			ui.show(function(): boolean
				return failed:get() ~= nil
			end, ui.text {
				function(): string
					return failed:get() or ""
				end,
				literal = true,
				wrap = true,
				color = "$danger",
				text_size = "$font.size.sm",
			}),
			ui.show(function(): boolean
				local b = bots:get()
				return b ~= nil and #b == 0 and failed:get() == nil
			end, ui.text { "No bots were found there.", literal = true, color = "$text.tertiary", text_size = "$font.size.sm" }),
			ui.show(function(): boolean
				local b = bots:get()
				return b ~= nil and #b > 1
			end, ui.row {
				w_full = true,
				justify = "end",
				ui.button {
					label = "Select all or none",
					h = 24,
					px = "$space.2",
					rounded = "$radius.md",
					bg = "#00000000",
					hover = { bg = "$hover" },
					on_click = function()
						local all = #chosen() < #(bots:peek() or {})
						local on: { [string]: boolean } = {}
						for _, b in bots:peek() or {} do
							on[b.name] = all
						end
						ticked:set(on)
					end,
					ui.text {
						function(): string
							return if #chosen() < #(bots:get() or {}) then "Select all" else "Select none"
						end,
						literal = true,
						color = "$text.secondary",
						text_size = "$font.size.xs",
					},
				},
			}),
			ui.scroll {
				w_full = true,
				max_h = 360,
				ui.each(ui.computed(function(): { any }
					return bots:get() or {}
				end), function(b: any): Node
					local function on(): boolean
						return ticked:get()[b.name] == true
					end
					return ui.button {
						label = b.name,
						w_full = true,
						h = 48,
						px = "$space.2",
						gap = "$space.3",
						items = "center",
						rounded = "$radius.lg",
						bg = "#00000000",
						hover = { bg = "$hover" },
						on_click = function()
							local now = table.clone(ticked:peek())
							now[b.name] = not now[b.name]
							ticked:set(now)
						end,
						ui.div {
							w = 16,
							h = 16,
							shrink = 0,
							rounded = "$radius.sm",
							border = 2,
							border_color = function(): string
								return if on() then "$accent" else "$text.tertiary"
							end,
							bg = function(): string
								return if on() then "$accent" else "#00000000"
							end,
						},
						Avatar { look = b.look, seed = b.name, size = 30 },
						ui.column {
							grow = 1,
							min_w = 0,
							gap = 0,
							ui.text { b.name, literal = true, truncate = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
							ui.text {
								`{if b.role and b.role ~= "" then b.role .. " · " else ""}{b.files or 0} files{if b.exists then " · adds to the one here" else ""}`,
								literal = true,
								truncate = true,
								color = "$text.tertiary",
								text_size = "$font.size.xs",
							},
						},
					}
				end, {
					w_full = true,
					key = function(b: any): string
						return b.name
					end,
				}),
			},
		},
	}
end)

local function import_grokbot()
	ui.call("kx.grokbot.choose"):after(function(from: any)
		if type(from) == "string" then
			import_from:set(from)
			import_open:set(true)
		end
	end, function(why: any)
		ds.toast.show { title = "Grok Bot couldn't be read", body = tostring(why), tone = "danger" }
	end)
end

local function import_template()
	store.import_template(function(id: string?, why: string?)
		if id then
			ui.after(100, function()
				local b = store.by_id(id)
				if b then
					store.show(b, "settings")
				end
			end)
		elseif why then
			ds.toast.show { title = "That template couldn't be imported", body = why, tone = "danger" }
		end
	end)
end

-- Where Buddies starts with none chosen: the buddies are in the sidebar (as
-- in Grok Bot); here is how to get more (a template, a template file, Grok
-- Bot) and what the clock has sent lately.
function M.board(): Node
	local starters: { [any]: any } = { w_full = true, gap = "$space.1", flex_wrap = true, items = "center" }
	for _, t in templates.list do
		table.insert(starters, starter(t))
	end
	return page {
		GrokImport {},
		ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			ui.text { "Buddies", literal = true, grow = 1, color = "$text.primary", text_size = "$font.size.xl", weight = "$font.weight.semibold" },
			ds.Button {
				tone = "ghost",
				size = "sm",
				on_click = import_template,
				ui.text { "Import a template", literal = true },
			},
			ds.Button {
				tone = "ghost",
				size = "sm",
				on_click = import_grokbot,
				ui.text { "Import from Grok Bot", literal = true },
			},
			ds.Button {
				size = "sm",
				on_click = function()
					store.create_open:set(true)
				end,
				Icon { name = "plus", size = 14 },
				ui.text { "New buddy", literal = true },
			},
		},
		ui.text {
			function(): string
				local n = #(store.list:get() or {})
				return if n == 0 then "No buddies yet. Start from a template, or bring yours across from Grok Bot." else "Your buddies are in the sidebar. Add another from a template:"
			end,
			literal = true,
			color = "$text.tertiary",
			text_size = "$font.size.sm",
		},
		ui.column { w_full = true, gap = "$space.2", heading("Templates"), ui.row(starters) },
		ui.column {
			w_full = true,
			gap = "$space.2",
			heading("Recent runs"),
			runs_list(function(): { store.Run }
				local out: { store.Run } = {}
				for i, r in store.runs:get() or {} do
					if i > 8 then
						break
					end
					table.insert(out, r)
				end
				return out
			end, "Nothing has run yet."),
		},
	}
end

-- ── A buddy's profile ───────────────────────────────────────────────────
-- One step of what it runs on: an agent, and which of its models.
local Step = ui.component(function(props: { step: store.Step, index: number, total: number, steps: State<{ store.Step }> }): Node
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
	local model_options: { any } = { { value = "", label = "Default model" } }
	for _, m in store.models(props.step.harness) do
		table.insert(model_options, m)
	end
	local function swap(other: number)
		local list = props.steps:peek()
		if not list[i] or not list[other] then
			return
		end
		local next = table.clone(list)
		next[i], next[other] = next[other], next[i]
		props.steps:set(next)
	end
	-- (Dragged by its grip onto another step: it takes that one's place.)
	local over = ui.use_state(false)
	return ui.row {
		w_full = true,
		px = "$space.3",
		py = "$space.2",
		gap = "$space.2",
		items = "center",
		border_sides = { 2, 0, 0, 0 },
		border_color = function(): string
			return if over:get() then "$accent" else "#00000000"
		end,
		on_drag = function(): any
			return { tag = "kx.step", from = i }
		end,
		drop_when = function(payload: any): boolean
			return type(payload) == "table" and payload.tag == "kx.step" and payload.from ~= i
		end,
		on_drag_over = function()
			if not over:peek() then
				over:set(true)
			end
		end,
		on_drag_leave = function()
			over:set(false)
		end,
		on_drop = function(payload: any)
			over:set(false)
			local next = table.clone(props.steps:peek())
			local moved = table.remove(next, payload.from)
			if moved then
				table.insert(next, i, moved)
				props.steps:set(next)
			end
		end,
		Icon { name = "grip" :: any, size = 14, color = "$text.tertiary" },
		ui.div {
			w = 22,
			h = 22,
			shrink = 0,
			rounded = "$radius.full",
			bg = "$hover",
			items = "center",
			justify = "center",
			if this and this.mark then ds.AgentMark { label = this.label, seed = this.harness, mark = this.mark, size = 13, bare = true } else ui.text { tostring(i), literal = true, color = "$text.tertiary", text_size = "$font.size.xs", tabular = true },
		},
		ui.div { grow = 1, min_w = 0, ds.Select { value = harness, options = agent_options } },
		ui.div { grow = 1, min_w = 0, ds.Select { value = model, options = model_options } },
		ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = if this and this.ok then "$success" else "$danger", tooltip = if this and this.ok then "Usable now" else (if this then this.why else "Not installed here") or "Not available" },
		ds.IconButton {
			icon = "alt-arrow-up" :: any,
			label = "Move up",
			size = "sm",
			tone = "quiet",
			disabled = i <= 1,
			on_click = function()
				swap(i - 1)
			end,
		},
		ds.IconButton {
			icon = "alt-arrow-down" :: any,
			label = "Move down",
			size = "sm",
			tone = "quiet",
			disabled = i >= props.total,
			on_click = function()
				swap(i + 1)
			end,
		},
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

local function steps_equal(a: { store.Step }, b: { store.Step }): boolean
	if #a ~= #b then
		return false
	end
	for i = 1, #a do
		if a[i].harness ~= b[i].harness or (a[i].model or "") ~= (b[i].model or "") then
			return false
		end
	end
	return true
end

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
	local extra = ui.use_state(given.extra)
	-- (The picker's own copy: as picked, while it rolls too.)
	local look_pick = ui.use_state({ shape = given.shape, color = given.color, face = given.face, extra = given.extra } :: Avatar.Look)
	local steps = ui.use_state(table.clone(b.models or {}) :: { store.Step })
	local saving = ui.use_state(false)

	local function look_now(): Avatar.Look
		return { shape = shape:get(), color = color:get(), face = face:get(), extra = extra:get() }
	end
	local dirty = ui.computed(function(): boolean
		if name:get() ~= b.name then
			return true
		elseif role:get() ~= (b.role or "") then
			return true
		elseif job:get() ~= (b.instructions or "") then
			return true
		elseif folder:get() ~= (b.folder or "") then
			return true
		elseif may:get() ~= (b.permissions or "draft") then
			return true
		elseif shape:get() ~= given.shape or color:get() ~= given.color or face:get() ~= given.face or extra:get() ~= given.extra then
			return true
		elseif not steps_equal(steps:get(), b.models or {}) then
			return true
		end
		return false
	end)
	local function discard()
		name:set(b.name)
		role:set(b.role or "")
		job:set(b.instructions or "")
		folder:set(b.folder or "")
		may:set(b.permissions or "draft")
		shape:set(given.shape)
		color:set(given.color)
		face:set(given.face)
		extra:set(given.extra)
		look_pick:set({ shape = given.shape, color = given.color, face = given.face, extra = given.extra })
		steps:set(table.clone(b.models or {}))
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
			look = { shape = shape:peek(), color = color:peek(), face = face:peek(), extra = extra:peek() },
			models = steps:peek(),
		}):after(function()
			saving:set(false)
			ds.toast.show { title = "Saved", tone = "success" }
		end, function(why: any)
			saving:set(false)
			ds.toast.show { title = "It couldn't be saved", body = tostring(why), tone = "danger" }
		end)
	end
	return ui.column {
		w_full = true,
		gap = "$space.6",
		Section {
			title = "Basics",
			Group {
				ds.Settings.Row { label = "Name", stacked = true, control = ds.TextField { value = name, placeholder = "Name", w_full = true } },
				ds.Settings.Row { label = "Role", stacked = true, control = ds.TextField { value = role, placeholder = "Researcher", w_full = true } },
				ds.Settings.Row {
					label = "Instructions",
					stacked = true,
					control = ds.TextArea { w_full = true, value = job, rows = 5, max_rows = 16, placeholder = "What should it do, and how should it report back?" },
				},
			},
		},
		Section {
			title = "Appearance",
			Group {
				ui.row {
					w_full = true,
					p = "$space.4",
					gap = "$space.6",
					items = "start",
					ui.column {
						shrink = 0,
						w = 112,
						pt = "$space.4",
						items = "center",
						Avatar { look = look_now, seed = b.id, size = 96, live = true },
					},
					ui.div {
						grow = 1,
						min_w = 0,
						Avatar.Picker {
							look = look_pick,
							seed = b.id,
							on_change = function(l: Avatar.Look)
								shape:set(l.shape or given.shape)
								color:set(l.color or given.color)
								face:set(l.face or given.face)
								extra:set(l.extra or given.extra)
							end,
						},
					},
				},
			},
		},
		Section {
			title = "Model",
			description = "Tried in order; the first one available runs. Drag to reorder.",
			Group {
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
								return Step { step = step, index = index, total = #steps:get(), steps = steps }
							end, {
								w_full = true,
								key = function(step: any, index: number): string
									return `{index}:{step.harness}:{step.model or ""}:{#steps:peek()}`
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
		Section {
			title = "Workspace",
			Group {
				ds.Settings.Row {
					label = "Folder",
					description = function(): string
						return `Optional: a project's folder. Its own notes and drafts live in {b.home or "its own folder"}.`
					end,
					stacked = true,
					control = ui.row {
						w_full = true,
						gap = "$space.2",
						items = "center",
						ui.div { grow = 1, min_w = 0, ds.TextField { value = folder, placeholder = "C:\\path\\to\\a\\project", w_full = true } },
						ds.Button {
							tone = "secondary",
							size = "sm",
							on_click = function()
								ui.call("kx.buddy.choose_folder", nil):after(function(path: any)
									if type(path) == "string" and path ~= "" then
										folder:set(path)
									end
								end, function() end)
							end,
							Icon { name = "folder", size = 14 },
							ui.text { "Browse…", literal = true },
						},
						ds.Button {
							tone = "ghost",
							size = "sm",
							on_click = function()
								pcall(ui.invoke, "kx.buddy.open_folder", b.id)
							end,
							ui.text { "Open home", literal = true },
						},
					},
				},
				ds.Settings.Row {
					label = "Permissions",
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
			px = "$space.4",
			py = "$space.3",
			gap = "$space.3",
			items = "center",
			rounded = "$radius.xl",
			bg = "$zeron.secondary.bg",
			border = 1,
			border_color = function(): string
				return if dirty:get() then "$accent" else "$border.subtle"
			end,
			ui.text {
				function(): string
					return if dirty:get() then "Unsaved changes" else ""
				end,
				literal = true,
				grow = 1,
				color = function(): string
					return if dirty:get() then "$text.primary" else "$text.tertiary"
				end,
				text_size = "$font.size.sm",
			},
			ui.show(function(): boolean
				return dirty:get()
			end, ds.Button {
				tone = "ghost",
				size = "sm",
				on_click = discard,
				ui.text { "Discard", literal = true },
			}),
			ds.Button {
				size = "sm",
				disabled = function(): boolean
					return saving:get() or not dirty:get()
				end,
				on_click = save,
				ui.text {
					function(): string
						return if saving:get() then "Saving…" else "Save changes"
					end,
					literal = true,
				},
			},
		},
	}
end)

-- ── A buddy's computer ──────────────────────────────────────────────────
-- A Cua Space on this PC (`cua spaces`), or in the cloud on Boat. On this PC
-- it first checks if any local Space is already there (so you can attach it
-- immediately) or lets you set up a new Space (`kx-<buddy_id>`).
local IMAGES = {
	{ value = "linux", label = "Ubuntu Linux (default)" },
	{ value = "omarchy", label = "Omarchy (Arch Linux desktop)" },
	{ value = "windows", label = "Windows" },
	{ value = "macos", label = "macOS" },
}
local COMP_STANDING: { [string]: string } = {
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
	local kind = b.computer and b.computer.kind
	-- (Nothing set up yet: None, and its setup opens only once one is chosen.)
	local where = ui.use_state(if kind == "boat" then "boat" elseif kind == "local" then "local" else "none")
	local image = ui.use_state("linux")
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
		-- (The buddy as it is now: attached or detached since this was drawn.)
		local c = store.computer_of(store.by_id(b.id) or b)
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
		elseif where:get() == "none" then
			return "off"
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
			ds.toast.show { title = `{b.name} has a Space`, tone = "success" }
		else
			ds.toast.show { title = "Its Space couldn't be made", body = why, tone = "danger" }
		end
	end
	local function forget_key()
		pcall(ui.invoke, "kx.key.remove", "boat")
		store.boats:set({})
		store.boats_read:set(false)
		ds.toast.show { title = "The Boat key is forgotten", tone = "success" }
	end
	return Section {
		title = "Computer",
		description = "Its own desktop, shown beside the chat.",
		Group {
			ui.row {
				w_full = true,
				px = "$space.3",
				py = "$space.2",
				gap = "$space.2",
				items = "center",
				ui.div {
					grow = 1,
					ds.Segmented {
						value = where,
						fill = true,
						options = { { value = "local", label = "This PC" }, { value = "boat", label = "Cloud" }, { value = "none", label = "None" } },
					},
				},
				ds.IconButton {
					icon = "refresh" :: any,
					label = "Check Spaces",
					size = "sm",
					tone = "quiet",
					on_click = read,
				},
			},
			ui.presence(state, function(which: string): Node
				if which == "off" then
					local attached = b.computer and (b.computer.kind == "local" or b.computer.kind == "boat")
					return ui.row {
						w_full = true,
						px = "$space.4",
						py = "$space.3",
						gap = "$space.3",
						items = "center",
						ui.text { "No desktop. It works with files and the web.", literal = true, grow = 1, color = "$text.tertiary", text_size = "$font.size.sm" },
						if attached
							then ds.Button {
								tone = "secondary",
								size = "sm",
								on_click = function()
									store.forget_computer(b)
									ds.toast.show { title = "Its computer is detached", body = "The Space itself is kept.", tone = "info" }
								end,
								ui.text { "Detach its computer", literal = true },
							}
							else nil,
					}
				elseif which == "none" then
					return ds.Settings.Row { label = "cua isn't installed", description = "A computer on this PC is a Cua Space. Install cua (cua.ai), then refresh this page." }
				elseif which == "busy" then
					return ds.Settings.Row {
						label = function(): string
							return busy:get() or "Working"
						end,
						description = "This takes a moment.",
					}
				elseif which == "reading" then
					return ui.column { w_full = true, ds.SkeletonRow { lead = 32, title = 140, line = 220, trail = 64 }, ds.SkeletonRow { lead = 32, title = 110, line = 180, trail = 64 } }
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
					-- (One field: the key, saved beside it; where to get one, under it.)
					return ui.column {
						w_full = true,
						px = "$space.3",
						py = "$space.3",
						gap = "$space.2",
						ui.row {
							w_full = true,
							gap = "$space.2",
							items = "center",
							ui.div { grow = 1, min_w = 0, ds.TextField { value = key, password = true, placeholder = "Paste your Boat key", w_full = true } },
							ds.Button {
								disabled = function(): boolean
									return key:get() == ""
								end,
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
								ui.text { "Save", literal = true },
							},
						},
						ui.row {
							gap = "$space.1",
							items = "center",
							ui.text { "Kept sealed on this PC.", literal = true, color = "$text.tertiary", text_size = "$font.size.xs" },
							ui.button {
								label = "Get a Boat key",
								on_click = function()
									pcall(ui.invoke, "open_url", "https://boat.dev")
								end,
								ui.text { "Get a key", literal = true, color = "$accent", text_size = "$font.size.xs" },
							},
						},
					}
				elseif which == "make" then
					local existing = store.computers:get() or {}
					return ui.column {
						w_full = true,
						gap = "$space.2",
						if #existing > 0
							then ui.column {
								w_full = true,
								ds.Settings.Row {
									label = `{#existing} existing Cua {if #existing == 1 then "Space" else "Spaces"} found on this PC`,
									description = nil,
								},
								ui.each(store.computers, function(sp: any): Node
									local sp_up = UP[sp.status or ""] == true
									-- (A QEMU VM that was stopped, not suspended: cua resumes
									-- those only from suspend, so it can't be turned on again.)
									local dead = not sp_up and sp.runtime == "qemu" and (sp.status == "stopped" or sp.state == "stopped")
									local function attach()
										store.attach_space(b, sp, function(ok: boolean, why: string?)
											busy:set(nil)
											store.read_computers()
											if ok then
												ds.toast.show { title = `{b.name} is connected to {sp.name}`, tone = "success" }
											else
												ds.toast.show { title = "Couldn't attach Space", body = why, tone = "danger" }
											end
										end)
									end
									return ui.row {
										w_full = true,
										px = "$space.3",
										py = "$space.2",
										gap = "$space.3",
										items = "center",
										ui.div { w = 7, h = 7, shrink = 0, rounded = "$radius.full", bg = if sp_up then "$success" else "$text.tertiary" },
										ui.column {
											grow = 1,
											min_w = 0,
											gap = 0,
											ui.text { sp.display_name or sp.name, literal = true, truncate = true, color = "$text.primary", text_size = "$font.size.md", weight = "$font.weight.medium" },
											ui.text {
												if dead
													then `{sp.os or "Linux"} · Stopped for good: this PC's cua can only wake a suspended VM`
													else `{sp.os or "Linux"} · {COMP_STANDING[sp.status or ""] or sp.status or "Stopped"}`,
												literal = true,
												truncate = true,
												color = "$text.tertiary",
												text_size = "$font.size.xs",
											},
										},
										ds.Button {
											tone = "secondary",
											size = "sm",
											disabled = dead,
											on_click = function()
												if sp_up then
													busy:set(`Attaching {sp.name}`)
													attach()
													return
												end
												-- (Turned on first: a Space that won't start is never
												-- attached, so a buddy isn't left with a dead computer.)
												busy:set(`Starting {sp.name}`)
												ui.call("kx.computer.start", sp.name):after(function()
													busy:set(`Attaching {sp.name}`)
													attach()
												end, function(start_err: any)
													busy:set(nil)
													store.read_computers()
													ds.toast.show { title = `{sp.name} wouldn't start`, body = `{tostring(start_err)}\nSet up a new Space below instead.`, tone = "danger" }
												end)
											end,
											ui.text { if dead then "Can't restart" elseif sp_up then "Attach" else "Start and attach", literal = true },
										},
									}
								end, {
									w_full = true,
									key = function(sp: any): string
										return `{sp.id}:{sp.status or ""}`
									end,
								}),
							}
							else nil,
						ds.Settings.Row {
							label = if #existing > 0 then "Or set up a new Space" else "No Spaces on this PC yet",
							description = "Named kx-" .. string.lower(b.id) .. ".",
							control = ui.div { w = 240, ds.Select { value = image, options = IMAGES } },
						},
						ui.row {
							w_full = true,
							px = "$space.3",
							py = "$space.2",
							ds.Button {
								tone = "secondary",
								size = "sm",
								on_click = function()
									busy:set("Creating a new Cua Space")
									store.make_computer(b, image:peek(), made)
								end,
								Icon { name = "plus", size = 14 },
								ui.text { "Create Space", literal = true },
							},
						},
					}
				elseif which == "make-boat" then
					return ui.column {
						w_full = true,
						ds.Settings.Row { label = "It has none yet", description = "A Linux desktop on Boat, billed while it runs. It stops on its own after an hour." },
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
				local sp_local = if not cloud() then store.computer_of(b) else nil
				return ui.column {
					w_full = true,
					ds.Settings.Row {
						label = store.computer_name(b),
						description = function(): string
							local _, standing = has()
							local word = COMP_STANDING[standing or ""] or standing or "Unknown"
							if cloud() then
								return `{word} · on Boat`
							end
							local os_word = if sp_local and sp_local.os then `{sp_local.os} · ` else ""
							return `{os_word}{word} · Cua Space`
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
							Icon { name = "monitor", size = 14 },
							ui.text { "Open desktop", literal = true },
						},
						ds.Button {
							tone = "secondary",
							size = "sm",
							on_click = function()
								if running then
									act("Stopping Space", "stop", "Stopped")
								else
									act("Starting Space", "start", "Started")
								end
							end,
							ui.text { if running then "Stop" else "Start", literal = true },
						},
						if not cloud()
							then ds.Button {
								tone = "ghost",
								size = "sm",
								on_click = function()
									store.forget_computer(b)
									ds.toast.show { title = "Detached Space from buddy", tone = "info" }
								end,
								ui.text { "Detach", literal = true },
							}
							else nil,
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
								act("Deleting Space", "remove", "Its Space is deleted", function()
									store.forget_computer(b)
								end)
							end,
							ui.text {
								function(): string
									return if sure:get() then "Delete Space and everything on it?" else "Delete"
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

local AutomationRow = ui.component(function(props: { a: store.Automation, on_edit: (store.Automation) -> () }): Node
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
				`{when_words(a)}{if a.enabled ~= false and a.next then " · next " .. tostring(os.date("%a %H:%M", a.next)) else " · off"}{if a.cap then " · cap " .. tostring(a.cap) .. "/d" else ""}`,
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
		ds.IconButton {
			icon = "pen" :: any,
			label = "Edit routine",
			size = "sm",
			tone = "quiet",
			on_click = function()
				props.on_edit(a)
			end,
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

local PRESETS = {
	{ label = "Every 15m", when = "every 15 minutes" },
	{ label = "Hourly", when = "every hour" },
	{ label = "Weekdays 9am", when = "every weekday at 9" },
	{ label = "Daily 8:30am", when = "daily at 8:30am" },
	{ label = "In 10m (once)", when = "in 10 minutes" },
}

local Automations = ui.component(function(props: { buddy: store.Buddy }): Node
	local b = props.buddy
	local editing_id = ui.use_state(nil :: string?)
	local sentence = ui.use_state("")
	local prompt = ui.use_state("")
	local title = ui.use_state("")
	local cap = ui.use_state("24")
	-- (Also started by a webhook: a URL and a key of its own.)
	local hook = ui.use_state(false)
	local read = ui.computed(function(): any
		return M.read_when(sentence:get())
	end)
	local list = ui.computed(function(): { store.Automation }
		return mine(b)
	end)
	local function clear_form()
		editing_id:set(nil)
		sentence:set("")
		prompt:set("")
		title:set("")
		cap:set("24")
		hook:set(false)
	end
	local function start_edit(a: store.Automation)
		editing_id:set(a.id)
		sentence:set(if a.kind == "webhook" then "" else string.lower(when_words(a)))
		prompt:set(a.prompt or "")
		title:set(a.title or "")
		cap:set(tostring(a.cap or 24))
		hook:set((a :: any).webhook == true)
	end
	local function save_routine()
		local w = read:peek()
		if not w and hook:peek() and string.match(sentence:peek(), "^%s*$") then
			-- (Only by its webhook: no clock.)
			w = { kind = "webhook" }
		end
		if not w then
			ds.toast.show { title = "Say when", body = "For example: every weekday at 9, every 30 minutes, or in 10 minutes.", tone = "warning" }
			return
		end
		if string.match(prompt:peek(), "^%s*$") then
			ds.toast.show { title = "Say what to send it", tone = "warning" }
			return
		end
		local name = title:peek()
		local cap_n = math.max(1, math.floor(tonumber(cap:peek()) or 24))
		local eid = editing_id:peek()
		ui.call("kx.automation.save", {
			id = eid,
			buddy = b.id,
			title = if string.match(name, "^%s*$") then string.sub(prompt:peek(), 1, 48) else name,
			prompt = prompt:peek(),
			kind = w.kind,
			minutes = w.minutes,
			time = w.time,
			days = w.days,
			at = w.at,
			cap = cap_n,
			webhook = hook:peek(),
			enabled = true,
		}):after(function()
			clear_form()
			ds.toast.show { title = if eid then "Routine updated" else "Routine added", tone = "success" }
		end, function(why: any)
			ds.toast.show { title = "It couldn't be saved", body = tostring(why), tone = "danger" }
		end)
	end
	local preset_row: { [any]: any } = { w_full = true, gap = "$space.1", flex_wrap = true, items = "center" }
	for _, p in PRESETS do
		table.insert(preset_row, ds.Button {
			tone = "ghost",
			size = "sm",
			on_click = function()
				sentence:set(p.when)
			end,
			ui.text { p.label, literal = true },
		})
	end
	return ui.column {
		w_full = true,
		gap = "$space.6",
		Section {
			title = "Routines",
			description = "Messages sent to its chat on a schedule, while Keplorayx is open.",
			Group {
				ui.column {
					w_full = true,
					ui.show(function(): boolean
						return #list:get() == 0
					end, ui.text { "No routines yet.", literal = true, px = "$space.3", py = "$space.3", color = "$text.tertiary", text_size = "$font.size.sm" }),
					ui.each(list, function(a: any): Node
						return AutomationRow { a = a, on_edit = start_edit }
					end, {
						w_full = true,
						key = function(a: any): string
							return `{a.id}:{a.title or ""}:{a.kind}:{a.minutes or 0}:{a.time or ""}:{tostring(a.enabled)}:{a.next or 0}:{a.cap or 24}`
						end,
					}),
				},
			},
		},
		Section {
			title = function(): string
				return if editing_id:get() then "Edit routine" else "Add a routine"
			end,
			Group {
				ds.Settings.Row {
					label = "When",
					description = function(): string
						local w = read:get()
						if string.match(sentence:get(), "^%s*$") then
							return "Say it in words or pick a preset: every weekday at 9, every 30 minutes, in 10 minutes."
						end
						return if w then `Read as: {when_words(w :: any)}.` else "That isn't a time I can read yet."
					end,
					stacked = true,
					control = ui.column {
						w_full = true,
						gap = "$space.2",
						ui.row(preset_row),
						ds.TextField { value = sentence, placeholder = "every weekday at 9", w_full = true },
					},
				},
				ds.Settings.Row {
					label = "What to send it",
					stacked = true,
					control = ds.TextArea { w_full = true, value = prompt, rows = 3, max_rows = 8, placeholder = "Summarise my open pull requests." },
				},
				ds.Settings.Row { label = "Name", description = "Optional.", stacked = true, control = ds.TextField { value = title, placeholder = "Morning summary", w_full = true } },
				ds.Settings.Row { label = "Daily cap", description = "Maximum runs per day (default 24).", stacked = true, control = ds.TextField { value = cap, placeholder = "24", w = 120 } },
				ds.Settings.Row {
					label = "Start it by webhook too",
					description = function(): string
						local a = editing_id:get() and store.automations:get() or {}
						for _, x in a do
							if x.id == editing_id:peek() and (x :: any).webhook_url then
								return `POST {(x :: any).webhook_url} with Authorization: Bearer <its key> (this PC only, while Keplorayx runs).`
							end
						end
						return "A URL on this PC that starts it, for scripts and tools here. Leave When empty for webhook only."
					end,
					control = ds.Switch { value = hook, label = "Start by webhook" },
				},
				ui.show(function(): boolean
					local id = editing_id:get()
					for _, x in store.automations:get() or {} do
						if x.id == id and (x :: any).webhook_key then
							return true
						end
					end
					return false
				end, ui.row {
					w_full = true,
					px = "$space.4",
					pb = "$space.3",
					gap = "$space.2",
					ds.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							for _, x in store.automations:peek() or {} do
								if x.id == editing_id:peek() then
									ui.clipboard.write((x :: any).webhook_url or "")
								end
							end
						end,
						ui.text { "Copy URL", literal = true },
					},
					ds.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							for _, x in store.automations:peek() or {} do
								if x.id == editing_id:peek() then
									ui.clipboard.write((x :: any).webhook_key or "")
								end
							end
						end,
						ui.text { "Copy key", literal = true },
					},
				}),
				ui.row {
					w_full = true,
					px = "$space.3",
					py = "$space.2",
					gap = "$space.2",
					ds.Button {
						size = "sm",
						on_click = save_routine,
						ui.text {
							function(): string
								return if editing_id:get() then "Save routine" else "Add routine"
							end,
							literal = true,
						},
					},
					ui.show(function(): boolean
						return editing_id:get() ~= nil
					end, ds.Button {
						tone = "ghost",
						size = "sm",
						on_click = clear_form,
						ui.text { "Cancel edit", literal = true },
					}),
				},
			},
		},
	}
end)

local Activity = ui.component(function(props: { buddy: store.Buddy }): Node
	local b = props.buddy
	local filter = ui.use_state("all")
	local shown = ui.computed(function(): { store.Run }
		local f = filter:get()
		local out: { store.Run } = {}
		for _, r in store.runs:get() or {} do
			if r.buddy == b.id and (f == "all" or r.status == f) then
				table.insert(out, r)
			end
		end
		return out
	end)
	return ui.column {
		w_full = true,
		gap = "$space.3",
		ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			ds.Segmented {
				value = filter,
				options = {
					{ value = "all", label = "All" },
					{ value = "sent", label = "Sent" },
					{ value = "failed", label = "Failed" },
					{ value = "skipped", label = "Skipped" },
				},
			},
		},
		runs_list(function(): { store.Run }
			return shown:get()
		end, "No matching runs yet."),
	}
end)

-- ── Its settings, in its drawer ─────────────────────────────────────────
local tab = store.page_tab
local deleting = ui.state("kx_buddy_deleting", false)

-- One skill of the library, on or off for this buddy.
local SkillRow = ui.component(function(props: { id: string, skill: { name: string, description: string } }): Node
	local function now(): store.Buddy?
		return store.by_id(props.id)
	end
	local start = now()
	local on = ui.use_state(if start then store.has_skill(start, props.skill.name) else true)
	ui.effect(function()
		local want = on:get()
		local b = now()
		if b and store.has_skill(b, props.skill.name) ~= want then
			store.set_skill(b, props.skill.name, want)
		end
	end)
	return ds.Settings.Row {
		label = props.skill.name,
		description = if props.skill.description ~= "" then props.skill.description else nil,
		control = ds.Switch { value = on, label = `Use {props.skill.name}` },
	}
end)

local function skills(b: store.Buddy): Node
	store.read_skills()
	return Section {
		title = "Skills",
		description = "Saved ways of doing a task, shared by every buddy. Ask one to save how it did something, and it lands here.",
		ui.show(function(): boolean
			return #store.skills:get() == 0
		end, ui.text { "None saved yet.", literal = true, color = "$text.tertiary", text_size = "$font.size.sm" }),
		ui.show(function(): boolean
			return #store.skills:get() > 0
		end, Group {
			ui.each(store.skills, function(s: any): Node
				return SkillRow { id = b.id, skill = s }
			end, {
				w_full = true,
				key = function(s: any): string
					return s.name
				end,
			}),
		}),
	}
end

-- The computer every buddy shares (as Grok Bot's bots do), and this one's
-- place on it.
local function shared(b: store.Buddy): Node
	return Group {
		ui.presence(function(): string
			local s = store.shared_computer:get()
			local mine = store.computer_of(b)
			local kind = b.computer and b.computer.kind or ""
			return `{s and s.id or ""}|{kind}|{mine and mine.id or ""}`
		end, function(): Node
			local s = store.shared_computer:peek()
			local kind = b.computer and b.computer.kind or ""
			local mine = if kind == "local" then store.computer_of(b) else nil
			if s and kind == "shared" then
				return ds.Settings.Row {
					label = "On the shared computer",
					description = `{s.name or s.id}: its files and signed-in sites are every buddy's.`,
					control = ds.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							store.save({ id = b.id, name = b.name, computer = { kind = "local" } })
						end,
						ui.text { "Give it its own", literal = true },
					},
				}
			elseif s then
				return ds.Settings.Row {
					label = "The shared computer",
					description = `{s.name or s.id}, which every buddy can use.`,
					control = ds.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							store.save({ id = b.id, name = b.name, computer = { kind = "shared" } })
						end,
						ui.text { "Use it", literal = true },
					},
				}
			elseif mine then
				return ds.Settings.Row {
					label = "Share this computer",
					description = "Let every buddy use it: new ones start on it.",
					control = ds.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							store.share_computer({ id = mine.id, name = mine.name })
						end,
						ui.text { "Share it", literal = true },
					},
				}
			end
			return ui.text { "Give this buddy a computer below; then it can be shared with the rest.", literal = true, wrap = true, px = "$space.4", py = "$space.3", color = "$text.tertiary", text_size = "$font.size.sm" }
		end, { w_full = true }),
	}
end

-- A buddy's settings, as its drawer shows them: who it is, its computer, its
-- routines and what they've done.
function M.Settings(b: store.Buddy): Node
	return ui.column {
		w_full = true,
		gap = "$space.4",
		pb = "$space.6",
		ds.Segmented {
			value = tab,
			fill = true,
			options = {
				{ value = "profile", label = "Profile" },
				{ value = "computer", label = "Computer" },
				{ value = "automations", label = "Routines" },
				{ value = "activity", label = "Runs" },
			},
		},
		ui.presence(function(): string
			return tab:get()
		end, function(which: string): Node
			if which == "computer" then
				return ui.column { w_full = true, gap = "$space.4", shared(b), Computer { buddy = b } }
			elseif which == "automations" then
				return Automations { buddy = b }
			elseif which == "activity" then
				return Activity { buddy = b }
			end
			return ui.column {
				w_full = true,
				gap = "$space.6",
				Profile { buddy = b },
				skills(b),
				ui.row {
					w_full = true,
					gap = "$space.2",
					ds.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							store.export(b, function(path: string?, why: string?)
								if path then
									ds.toast.show { title = "Saved as a template", body = path, tone = "success" }
								elseif why then
									ds.toast.show { title = "It couldn't be saved", body = why, tone = "danger" }
								end
							end)
						end,
						ui.text { "Save as a template", literal = true },
					},
					ui.div { grow = 1 },
					ds.Button {
						tone = "ghost",
						size = "sm",
						on_click = function()
							deleting:set(true)
						end,
						ui.text { "Remove this buddy", literal = true, color = "$danger" },
					},
				},
			}
		end, { w_full = true }),
		ds.Dialog {
			open = deleting,
			title = `Remove {b.name}?`,
			body = "Its routines go with it and its chat is archived. Its folder, with what it wrote, stays on disk.",
			tone = "danger",
			confirm = {
				label = "Remove",
				on_click = function()
					deleting:set(false)
					if b.chat then
						sessions.archive(b.chat)
					end
					store.remove(b.id)
					store.inspector_tab:set("activity")
					go_board()
				end,
			},
		},
	}
end

-- A buddy's settings as a page: where an older link to its page leads.
function M.buddy(): Node
	return ui.presence(function(): string
		local b = store.by_id(store.selected:get())
		return if b then b.id else ""
	end, function(id: string): Node
		local b = store.by_id(id)
		if not b then
			return page { ds.EmptyState { icon = "users-group-rounded", size = "sm", title = "That buddy is gone", body = "The others are in the sidebar." } }
		end
		return page { ui.text { b.name, literal = true, color = "$text.primary", text_size = "$font.size.xl", weight = "$font.weight.semibold" }, M.Settings(b) }
	end, { w_full = true, h_full = true })
end

return M
