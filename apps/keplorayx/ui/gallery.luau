--!strict
-- Orbit's workbench: every token and component, a page each. The header
-- switches night and dawn, frosted and solid, and reduced motion. Save any
-- kx/ module or theme file and the page restyles in place.
--
-- Run: cargo run -p keplorayx -- gallery

ui.theme.strict("warn")
local kx = require("@kx")
kx.setup()

local page = ui.persist("gallery.page", "tokens")
local scheme = ui.persist("gallery.scheme", "orbit-night")
-- Development: the page and theme the host was asked for (tools/shot.ps1).
local dev_page = (ui.shared("dev.page", "") :: any):peek() :: string
local dev_theme = (ui.shared("dev.theme", "") :: any):peek() :: string
if dev_page ~= "" then
	page:set(dev_page)
end
if dev_theme ~= "" then
	scheme:set(dev_theme)
end
ui.effect(function()
	ui.theme.set(scheme:get())
end)

local PAGES = {
	{ id = "tokens", title = "Tokens" },
	{ id = "type", title = "Type" },
	{ id = "icons", title = "Icons" },
	{ id = "buttons", title = "Buttons" },
	{ id = "controls", title = "Controls" },
	{ id = "rows", title = "Rows and agents" },
	{ id = "surfaces", title = "Surfaces" },
	{ id = "overlays", title = "Overlays" },
	{ id = "motion", title = "Motion" },
}

local function section(title: string, ...: any): Node
	return ui.column {
		w_full = true,
		gap = "$space.3",
		kx.Heading { title, literal = true },
		...,
	}
end

-- ── Tokens ─────────────────────────────────────────────────────────────

local function swatch(token: string, name: string): Node
	return ui.column {
		w = 112,
		shrink = 0,
		gap = "$space.1",
		ui.div {
			h = 48,
			w_full = true,
			rounded = "$radius.md",
			bg = token,
			border = 1,
			border_color = "$border.subtle",
		},
		kx.Caption { name, literal = true },
	}
end

local function swatches(list: { { string } }): Node
	local out: { Node } = {}
	for _, s in list do
		table.insert(out, swatch(s[1], s[2]))
	end
	return ui.row { flex_wrap = true, gap = "$space.3", table.unpack(out) }
end

local function tokens_page(): Node
	local agents: { { string } } = {}
	local ids = {}
	for id in kx.tokens.AGENTS do
		table.insert(ids, id)
	end
	table.sort(ids)
	for _, id in ids do
		table.insert(agents, { `$kx.agent.{id}`, id })
	end
	local radii: { Node } = {}
	for _, r in { "sm", "md", "lg", "xl" } do
		table.insert(radii, ui.column {
			gap = "$space.1",
			items = "center",
			ui.div { w = 72, h = 48, rounded = `$radius.{r}`, bg = "$surface.raised", border = 1, border_color = "$border" },
			kx.Caption { r, literal = true },
		})
	end
	local spaces: { Node } = {}
	for i = 1, 8 do
		table.insert(spaces, ui.row {
			gap = "$space.3",
			items = "center",
			ui.div { w = `$space.{i}`, h = 12, rounded = "$radius.sm", bg = "$accent" },
			kx.Caption { `space.{i}`, literal = true },
		})
	end
	return ui.column {
		gap = "$space.7",
		section(
			"Surfaces",
			swatches {
				{ "$background", "background" },
				{ "$surface.sunken", "sunken" },
				{ "$surface.base", "base" },
				{ "$surface.raised", "raised" },
				{ "$surface.overlay", "overlay" },
			}
		),
		section(
			"Light and ink",
			swatches {
				{ "$text.primary", "text" },
				{ "$text.secondary", "secondary" },
				{ "$text.tertiary", "tertiary" },
				{ "$accent", "accent" },
				{ "$kx.attention", "attention" },
				{ "$danger", "danger" },
				{ "$success", "success" },
			}
		),
		section("Agents", swatches(agents)),
		section("Lens radii", ui.row { gap = "$space.4", table.unpack(radii) }),
		section("Space", ui.column { gap = "$space.2", table.unpack(spaces) }),
	}
end

-- ── Type ───────────────────────────────────────────────────────────────

local function type_page(): Node
	return ui.column {
		gap = "$space.5",
		max_w = kx.MEASURE,
		kx.Title { "Good evening, Ravi", literal = true },
		kx.Heading { "Three agents are working", literal = true },
		kx.Body {
			"Type does the work here. Hierarchy comes from weight and space, not boxes; reading text keeps a generous line and a measure of about sixty-eight characters, so a long reply reads like a page, not a log.",
			literal = true,
		},
		kx.Label { "Model", literal = true },
		kx.Caption { "Updated 2 minutes ago", literal = true },
		kx.Hint { "keplorayx · main · 3 files changed", literal = true },
		kx.Mono { "cargo test -p keplorayx  # 4 passed", literal = true },
		ui.row { gap = "$space.3", kx.Numeric { "00:42", literal = true }, kx.Numeric { "$0.18", literal = true } },
	}
end

-- ── Icons ──────────────────────────────────────────────────────────────

local function icons_page(): Node
	local cells: { Node } = {}
	for _, name in kx.icons.names() do
		table.insert(cells, ui.column {
			w = 96,
			h = 76,
			gap = "$space.2",
			items = "center",
			justify = "center",
			rounded = "$radius.md",
			hover = { bg = "$hover" },
			tooltip = name,
			color = "$text.primary",
			kx.Icon { name = name, size = 16 },
			kx.Hint { name, literal = true, truncate = true, max_w = 88 },
		})
	end
	local sizes: { Node } = {}
	for _, s in { 12, 14, 16, 20, 24, 32 } do
		table.insert(sizes, ui.column {
			gap = "$space.2",
			items = "center",
			color = "$text.primary",
			kx.Icon { name = "orbit", size = s },
			kx.Hint { `{s}`, literal = true },
		})
	end
	local marks: { Node } = {}
	for _, name in kx.icons.mark_names() do
		table.insert(marks, ui.column {
			w = 96,
			h = 64,
			gap = "$space.2",
			items = "center",
			justify = "center",
			kx.Icon { mark = name, size = 18 },
			kx.Hint { name, literal = true, truncate = true, max_w = 88 },
		})
	end
	return ui.column {
		gap = "$space.6",
		section("Optical weight", ui.row { gap = "$space.5", items = "end", table.unpack(sizes) }),
		section("Glyphs", ui.row { flex_wrap = true, gap = "$space.1", table.unpack(cells) }),
		section("Marks", ui.row { flex_wrap = true, gap = "$space.1", table.unpack(marks) }),
	}
end

-- ── Buttons ────────────────────────────────────────────────────────────

local saving = ui.state("gallery.saving", false)

local function buttons_page(): Node
	local tones: { Node } = {}
	for _, tone in { "primary", "secondary", "ghost", "attention", "danger" } do
		local sizes: { Node } = {}
		for _, size in { "xs", "sm", "md", "lg" } do
			table.insert(sizes, kx.Button { tone = tone :: any, size = size :: any, `{tone}` })
		end
		table.insert(tones, ui.row { gap = "$space.3", items = "center", table.unpack(sizes) })
	end
	return ui.column {
		gap = "$space.6",
		section("Tones and sizes", ui.column { gap = "$space.3", table.unpack(tones) }),
		section(
			"With icons",
			ui.row {
				gap = "$space.3",
				items = "center",
				kx.Button { tone = "secondary", kx.Icon { name = "plus" }, "New thread" },
				kx.Button { tone = "ghost", icon = true, label = "Search", kx.Icon { name = "magnifer" } },
				kx.Button { tone = "ghost", icon = true, size = "sm", label = "More", kx.Icon { name = "more-horizontal" } },
				kx.Button { shape = "pill", kx.Icon { name = "send-up" }, "Send" },
			}
		),
		section(
			"Busy",
			ui.row {
				gap = "$space.3",
				kx.Button {
					busy = saving,
					on_click = function()
						saving:set(true)
						ui.after(1800, function()
							saving:set(false)
						end)
					end,
					"Save",
				},
				kx.Caption { "Press it: the label holds its width and breathes.", literal = true },
			}
		),
	}
end

-- ── Controls ───────────────────────────────────────────────────────────

local name = ui.state("gallery.name", "")
local reduce = kx.motion.reduce
local frosted = kx.material.frosted
local density = ui.state("gallery.density", "comfortable")

local function controls_page(): Node
	return ui.column {
		gap = "$space.6",
		max_w = 480,
		section("Field", kx.Field { value = name, placeholder = "Your name", label = "Your name", w = 320 }),
		section(
			"Switches",
			ui.column {
				gap = "$space.3",
				kx.Switch { value = frosted, label = "Frosted glass", text = kx.Label { "Frosted glass", literal = true } },
				kx.Switch {
					value = reduce,
					label = "Reduce motion",
					text = kx.Label { "Reduce motion", literal = true },
					hint = kx.Caption { "Moves land at once; fades stay short.", literal = true },
				},
			}
		),
		section(
			"Segmented",
			kx.Segmented {
				value = density,
				label = "Density",
				options = {
					{ label = "Compact", value = "compact" },
					{ label = "Comfortable", value = "comfortable" },
					{ label = "Roomy", value = "roomy" },
				},
				width = 104,
			}
		),
		section(
			"Pill",
			kx.Pill {
				w = 460,
				ui.column {
					grow = 1,
					px = "$space.2",
					py = "$space.1",
					kx.Caption { "Ask, search or run a command…", literal = true },
				},
				kx.Button { shape = "pill", icon = true, size = "sm", label = "Send", kx.Icon { name = "arrow-up" } },
			}
		),
	}
end

-- ── Rows and agents ────────────────────────────────────────────────────

local chosen = ui.state("gallery.row", "b")

local function rows_page(): Node
	local function row(id: string, title: string, agent: string, live: boolean, needs: boolean): Node
		return kx.Row {
			selected = function()
				return chosen:get() == id
			end,
			on_click = function()
				chosen:set(id)
			end,
			kx.AgentMark { agent = agent, live = function()
				return live
			end, needs_you = function()
				return needs
			end },
			ui.text { title, literal = true, grow = 1, truncate = true },
			if needs then kx.Hint { "needs you", literal = true, color = "$kx.attention" } else kx.Hint { "2m", literal = true },
		}
	end
	return ui.column {
		gap = "$space.6",
		max_w = 360,
		section(
			"A list",
			ui.column {
				gap = "$kx.space.hair",
				p = "$space.1",
				rounded = "$radius.lg",
				bg = "$surface.base",
				row("a", "Fix the flaky login test", "claude_code", true, false),
				row("b", "Port the notch to zgx", "codex", false, false),
				row("c", "Approve: run migrations", "gemini", false, true),
				row("d", "Summarise yesterday's PRs", "open_code", false, false),
			}
		),
		section(
			"Keys",
			ui.row { gap = "$space.1", kx.Kbd { "Ctrl" }, kx.Kbd { "K" } }
		),
		section(
			"Empty",
			kx.Empty {
				icon = "constellation",
				title = "Nothing running",
				hint = "Start a thread and its agent will appear here, in its own light.",
				action = kx.Button { tone = "secondary", size = "sm", "New thread" },
			}
		),
	}
end

-- ── Surfaces ───────────────────────────────────────────────────────────

local function surfaces_page(): Node
	return ui.row {
		gap = "$space.5",
		flex_wrap = true,
		kx.Card { w = 260, gap = "$space.2", kx.Label { "Raised", literal = true }, kx.Caption { "Lit along its top edge.", literal = true } },
		kx.Card { w = 260, level = "base", gap = "$space.2", kx.Label { "Base", literal = true }, kx.Caption { "Quieter.", literal = true } },
		kx.Card { w = 260, level = "sunken", gap = "$space.2", kx.Label { "Sunken", literal = true }, kx.Caption { "Set into the window.", literal = true } },
		kx.Float {
			w = 232,
			kx.Row { size = "sm", kx.Icon { name = "pen" }, ui.text { "Rename", literal = true, grow = 1 } },
			kx.Row { size = "sm", kx.Icon { name = "archive-minimalistic" }, ui.text { "Archive", literal = true, grow = 1 } },
			kx.Rule { my = "$space.1" },
			kx.Row { size = "sm", color = "$danger", kx.Icon { name = "trash-bin-minimalistic" }, ui.text { "Delete", literal = true, grow = 1 } },
		},
	}
end

-- ── Overlays ───────────────────────────────────────────────────────────

local menu_open = ui.state("gallery.menu", false)
local dialog_open = ui.state("gallery.dialog", false)

local function overlays_page(): Node
	return ui.column {
		gap = "$space.5",
		ui.row {
			gap = "$space.3",
			kx.Menu {
				open = menu_open,
				label = "Thread",
				anchor = kx.Button { tone = "secondary", on_click = ui.toggle(menu_open), "Menu" },
				kx.MenuItem { "Rename", icon = "pen", on_click = function() end },
				kx.MenuItem { "Pin", icon = "pin", on_click = function() end },
				kx.MenuItem { "Archive", icon = "archive-minimalistic", on_click = function() end },
			},
			kx.Button { tone = "secondary", on_click = ui.set(dialog_open, true), "Dialog" },
			kx.Button {
				tone = "secondary",
				on_click = function()
					kx.toast { title = "Copied to clipboard" }
				end,
				"Toast",
			},
			kx.Button {
				tone = "secondary",
				on_click = function()
					kx.toast { title = "Codex needs you", body = "Approve a command", kind = "attention" }
				end,
				"Attention toast",
			},
			kx.Tooltip(kx.Button { tone = "ghost", "Hover me" }, "A quiet hint"),
		},
		kx.Dialog {
			open = dialog_open,
			title = "Archive this thread?",
			body = "It leaves the sidebar. You can find it again in Search.",
			actions = {
				kx.Button { tone = "ghost", on_click = ui.set(dialog_open, false), "Cancel" },
				kx.Button { on_click = ui.set(dialog_open, false), "Archive" },
			},
		},
	}
end

-- ── Motion ─────────────────────────────────────────────────────────────

local far = ui.state("gallery.far", false)

local function motion_page(): Node
	local x = ui.computed(function()
		return if far:get() then 320 else 0
	end)
	local Live = ui.component(function()
		return ui.row {
			gap = "$space.4",
			items = "center",
			kx.AgentMark { agent = "claude_code", size = 10, live = function()
				return true
			end },
			kx.AgentMark { agent = "codex", size = 10, live = function()
				return true
			end },
			kx.AgentMark { agent = "gemini", size = 10, live = function()
				return true
			end },
			kx.Caption { "Every live thing breathes on one clock.", literal = true },
		}
	end)
	return ui.column {
		gap = "$space.6",
		section(
			"Orbit's curve",
			ui.column {
				gap = "$space.3",
				ui.div {
					w = 360,
					h = 40,
					rounded = "$radius.lg",
					bg = "$surface.base",
					ui.div {
						w = 40,
						h = 40,
						rounded = "$radius.lg",
						bg = "$accent",
						x = kx.motion.tween(x, kx.motion.move(320)),
					},
				},
				kx.Button { tone = "secondary", on_click = ui.toggle(far), "Move" },
			}
		),
		section("The breath", Live {}),
	}
end

-- ── The workbench ──────────────────────────────────────────────────────

local function nav(): Node
	local items: { Node } = {}
	for _, p in PAGES do
		table.insert(items, kx.Row {
			size = "sm",
			selected = function()
				return page:get() == p.id
			end,
			on_click = function()
				page:set(p.id)
			end,
			ui.text { p.title, literal = true, grow = 1 },
		})
	end
	return ui.column {
		w = 220,
		h_full = true,
		gap = "$kx.space.hair",
		px = "$space.2",
		pt = "$kx.titlebar",
		bg = "$surface.sunken",
		window_drag = true,
		table.unpack(items),
	}
end

local header = ui.row {
	h = "$kx.titlebar",
	w_full = true,
	window_drag = true,
	items = "center",
	px = "$space.5",
	gap = "$space.3",
	pr = function()
		local area, buttons = ui.window.titlebar_area:get(), ui.window.controls:get()
		local inset = if area and buttons and buttons.x >= area.x + area.width then buttons.width + 12 else 12
		return ui.raw(inset, "clears the window's caption buttons, which the system places")
	end,
	ui.row { gap = "$space.2", items = "center", color = "$accent", kx.Icon { name = "orbit", size = 18 }, kx.Label { "Orbit", literal = true } },
	ui.div { grow = 1 },
	kx.Segmented {
		value = scheme,
		label = "Theme",
		width = 72,
		options = { { label = "Night", value = "orbit-night" }, { label = "Dawn", value = "orbit-dawn" } },
	},
}

local pages: { [string]: Node } = {
	tokens = ui.column { w_full = true, tokens_page() },
	type = ui.column { w_full = true, type_page() },
	icons = ui.column { w_full = true, icons_page() },
	buttons = ui.column { w_full = true, buttons_page() },
	controls = ui.column { w_full = true, controls_page() },
	rows = ui.column { w_full = true, rows_page() },
	surfaces = ui.column { w_full = true, surfaces_page() },
	overlays = ui.column { w_full = true, overlays_page() },
	motion = ui.column { w_full = true, motion_page() },
}

return ui.row {
	w_full = true,
	h_full = true,
	relative = true,
	bg = kx.material.window_bg(),
	nav(),
	ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		header,
		ui.scroll {
			grow = 1,
			w_full = true,
			ui.column {
				w_full = true,
				p = "$space.6",
				ui.switch(page, pages, nil),
			},
		},
	},
	kx.Toasts {},
}
