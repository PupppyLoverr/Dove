-- The 42 authored tokens a theme file may set, and everything else derived
-- from them. src/authored.rs owns the registry's *rules* (what kind each
-- token is, its clamp, its contrast lift, and the rejection of a theme that
-- names a derived one); this module owns what each derived token is, and
-- registers the result as the window's theme.
--
-- Why the cut: a theme of 267 leaves is one nobody writes correctly by
-- hand and one that can disappear a selected row by setting its fill to
-- nothing. With 42 authored and the rest derived, the worst a theme can do
-- is choose bad colours. §KEPL-11 3.1 has the set and 3.2 the fallback.
--
--   authored.derive(values, scheme)   the authored 42 as a flat token table, plus every derived one
--   authored.wear()                  both schemes, from the reports src/authored.rs checked
--   authored.sized(size)             the scale at another body size (Settings › Appearance)
--   authored.report()                what the check said, for the Studio and for `themes.authored`

local M = {}

-- The 42, in their nine groups, in the order a file writes them.
M.SPEC = {
	{ group = "surface", names = { "surface.window", "surface.base", "surface.raised", "surface.overlay" } },
	{ group = "ink", names = { "ink.primary", "ink.secondary", "ink.tertiary" } },
	{ group = "line", names = { "line.hairline", "line.strong" } },
	{ group = "accent", names = { "accent", "accent.ink" } },
	{ group = "tone", names = { "tone.ok", "tone.warn", "tone.danger", "tone.info" } },
	{ group = "attention", names = { "attention", "attention.ink" } },
	{ group = "agent", names = { "agent.1", "agent.2", "agent.3", "agent.4", "agent.5", "agent.6", "agent.7", "agent.8" } },
	{ group = "font", names = { "font.sans", "font.mono", "font.display", "font.size", "font.line", "font.weight", "font.strong" } },
	{
		group = "metric",
		names = {
			"metric.unit",
			"metric.radius",
			"metric.row",
			"metric.control",
			"metric.gutter",
			"metric.conversation",
			"metric.page",
			"metric.duration",
			"metric.ease",
			"metric.blur",
		},
	},
}

-- Every name here is computed, never written: a theme file that sets one is
-- rejected by src/authored.rs, which is the half of this guarantee a person
-- can see in the error.
M.DERIVED = {
	"background", "glass.tint", "glass.blur", "popover.bg", "surface.sunken",
	"text.primary", "text.secondary", "text.tertiary", "text.disabled", "ink.disabled",
	"border", "border.subtle", "border.strong",
	"accent.hover", "accent.text", "accent.wash", "accent.active",
	"danger", "warning", "success", "info",
	"tone.ok.ink", "tone.ok.wash", "tone.ok.edge", "tone.warn.ink", "tone.warn.wash", "tone.warn.edge",
	"tone.danger.ink", "tone.danger.wash", "tone.danger.edge", "tone.info.ink", "tone.info.wash", "tone.info.edge",
	"attention.wash", "attention.edge",
	"hover", "pressed", "selection", "selection.item", "selection.inactive", "focus.ring",
	"scrollbar.track", "scrollbar.thumb", "scrollbar.thumb.active", "backdrop",
	"shadow.sm", "shadow.md", "shadow.lg",
	"font.size.xxs", "font.size.xs", "font.size.sm", "font.size.md", "font.size.lg", "font.size.lg2", "font.size.xl", "font.size.xxl",
	"font.line.xxs", "font.line.xs", "font.line.sm", "font.line.md", "font.line.lg", "font.line.lg2", "font.line.xl", "font.line.xxl",
	"font.weight.regular", "font.weight.medium", "font.weight.semibold", "font.weight.bold",
	"space.1", "space.2", "space.3", "space.4", "space.5", "space.6", "space.7", "space.8",
	"radius.xs", "radius.sm", "radius.md", "radius.lg", "radius.xl", "radius.full",
	"control.height", "density.row", "density.row.compact", "density.control", "density.gutter", "band.height",
	"measure.conversation", "measure.page",
	"motion.duration.fast", "motion.duration.normal", "motion.duration.slow",
	"motion.ease.standard", "motion.ease.enter", "motion.ease.exit",
	"material.blur", "material.scrim",
	"zeron.window", "zeron.selected.bg", "zeron.selected.ring", "zeron.selected.flat.bg", "zeron.menu.selected",
	"zeron.composer.tint", "zeron.composer.solid", "zeron.composer.edge",
	"zeron.primary.bg", "zeron.primary.text", "zeron.destructive.bg", "zeron.destructive.text",
	"zeron.diff.added", "zeron.diff.removed",
	"zeron.git.modified", "zeron.git.added", "zeron.git.deleted",
	"zeron.pr.open", "zeron.pr.merged", "zeron.pr.closed",
	"zeron.status.working", "zeron.status.input", "zeron.status.approval", "zeron.status.done", "zeron.status.failed", "zeron.status.limited",
	"zeron.status.dot.done", "zeron.status.dot.failed", "zeron.attention", "kx.display", "kx.attention",
}

local function round(n: number): number
	return math.floor(n + 0.5)
end

--------------------------------------------------------------------------
-- Colours, as the registered theme needs them: concrete, not expressions.
--
-- A theme file may write `alpha({ink.primary}, 0.6)`; what the window is
-- registered with is resolved, because a registered theme's expressions are
-- read in the order the table happens to have (a Luau table's is not the
-- file's), and a token that refers to one registered beside it can resolve
-- to nothing. Concrete values are the whole answer to that, and the
-- derivation stays one line each either way.
--------------------------------------------------------------------------

type Rgba = { r: number, g: number, b: number, a: number }

local function parse(hex: string): Rgba
	local h = string.match(hex, "^#(%x+)$") or ""
	if #h == 3 then
		h = string.rep(string.sub(h, 1, 1), 2) .. string.rep(string.sub(h, 2, 2), 2) .. string.rep(string.sub(h, 3, 3), 2)
	elseif #h == 4 then
		h = string.rep(string.sub(h, 1, 1), 2) .. string.rep(string.sub(h, 2, 2), 2) .. string.rep(string.sub(h, 3, 3), 2) .. string.rep(string.sub(h, 4, 4), 2)
	end
	local n = tonumber(h, 16) or 0
	local function byte(at: number): number
		return math.floor(n / 256 ^ at) % 256 / 255
	end
	if #h == 6 then
		return { r = byte(2), g = byte(1), b = byte(0), a = 1 }
	end
	return { r = byte(3), g = byte(2), b = byte(1), a = byte(0) }
end

local function hex_of(c: Rgba): string
	local function byte(v: number): number
		return math.clamp(round(v * 255), 0, 255)
	end
	if c.a >= 0.999 then
		return string.format("#%02x%02x%02x", byte(c.r), byte(c.g), byte(c.b))
	end
	return string.format("#%02x%02x%02x%02x", byte(c.r), byte(c.g), byte(c.b), byte(c.a))
end

-- `colour` at `amount` of its own opacity.
local function alpha(colour: string, amount: number): string
	local c = parse(colour)
	c.a = c.a * amount
	return hex_of(c)
end

-- `colour` moved `amount` of the way toward `toward`.
local function mix(colour: string, toward: string, amount: number): string
	local a, b = parse(colour), parse(toward)
	return hex_of({
		r = a.r + (b.r - a.r) * amount,
		g = a.g + (b.g - a.g) * amount,
		b = a.b + (b.b - a.b) * amount,
		a = a.a + (b.a - a.a) * amount,
	})
end

-- `values` (the authored 42, concrete: a colour is `#rrggbb`, a length is a
-- number) as the theme table the window registers. Every line here is the
-- derivation: change one of the 42 and this is what moves.
function M.derive(values: { [string]: any }, scheme: string): { [string]: any }
	local dark = scheme ~= "light"
	local a = values
	local ink, accent = a["ink.primary"], a["accent"]
	local size = a["font.size"] :: number
	local line = a["font.line"] :: number
	local unit = a["metric.unit"] :: number
	local radius = a["metric.radius"] :: number
	local duration = a["metric.duration"] :: number
	local t: { [string]: any } = {}

	for _, group in ipairs(M.SPEC) do
		for _, name in ipairs(group.names) do
			t[name] = a[name]
		end
	end

	-- Surfaces and glass.
	t.background = a["surface.window"]
	t["glass.tint"] = a["surface.window"]
	t["glass.blur"] = a["metric.blur"]
	t["surface.sunken"] = alpha(ink, if dark then 0.05 else 0.04)
	t["popover.bg"] = a["surface.overlay"]
	t["material.blur"] = a["metric.blur"]
	t["material.scrim"] = alpha(a["surface.window"], 0.35)
	t.backdrop = if dark then "#00000059" else "#0000002e"

	-- Inks and lines.
	t["text.primary"] = ink
	t["text.secondary"] = a["ink.secondary"]
	t["text.tertiary"] = a["ink.tertiary"]
	t["text.disabled"] = alpha(ink, 0.28)
	t["ink.disabled"] = alpha(ink, 0.28)
	t.border = alpha(ink, if dark then 0.11 else 0.10)
	t["border.subtle"] = a["line.hairline"]
	t["border.strong"] = a["line.strong"]

	-- The accent, and what follows it rather than being chosen.
	t.accent = accent
	t["accent.hover"] = mix(accent, if dark then "#ffffff" else "#000000", 0.12)
	t["accent.text"] = a["accent.ink"]
	t["accent.wash"] = alpha(accent, 0.14)
	t["accent.active"] = mix(accent, if dark then "#000000" else "#ffffff", 0.14)

	-- The tones, and the four ways each is carried.
	t.success, t.warning, t.danger, t.info = a["tone.ok"], a["tone.warn"], a["tone.danger"], a["tone.info"]
	for _, pair in ipairs({ { "ok", "success" }, { "warn", "warning" }, { "danger", "danger" }, { "info", "info" } }) do
		local tone, role = pair[1] :: string, pair[2] :: string
		local source = "tone." .. tone
		local wash = alpha(a[source], 0.12)
		local edge = alpha(a[source], 0.28)
		t["tone." .. tone .. ".ink"] = a[source]
		t["tone." .. tone .. ".wash"] = wash
		t["tone." .. tone .. ".edge"] = edge
		-- (The names the app's own components read.)
		t["zeron.tone." .. role .. ".ink"] = a[source]
		t["zeron.tone." .. role .. ".wash"] = wash
		t["zeron.tone." .. role .. ".edge"] = edge
	end

	t["attention"] = a["attention"]
	t["attention.ink"] = a["attention.ink"]
	t["attention.wash"] = alpha(a["attention"], 0.14)
	t["attention.edge"] = alpha(a["attention"], 0.30)

	for n = 1, 8 do
		t["agent." .. n] = a["agent." .. n]
		t["zeron.agent." .. n] = a["agent." .. n]
	end

	-- What a pointer, a choice and the keys look like: all from ink and
	-- accent, so no theme can make a selected row invisible.
	t.hover = alpha(ink, 0.05)
	t.pressed = alpha(ink, 0.09)
	t.selection = alpha(accent, 0.32)
	t["selection.item"] = alpha(accent, 0.15)
	t["selection.inactive"] = alpha(ink, 0.12)
	t["focus.ring"] = alpha(accent, 0.6)
	t["scrollbar.track"] = "#00000000"
	t["scrollbar.thumb"] = alpha(ink, 0.26)
	t["scrollbar.thumb.active"] = alpha(ink, 0.44)

	t["shadow.sm"] = if dark then "0 1 2 0 #00000059" else "0 1 2 0 #0000000f"
	t["shadow.md"] = if dark then "0 4 12 0 #00000066" else "0 4 12 0 #00000014"
	t["shadow.lg"] = if dark then "0 12 32 0 #00000080" else "0 12 32 0 #0000001f"

	-- Type: every step from the one size, every line from the one line,
	-- every weight from the two. `xxs` (a step under 11 at the base size) and
	-- `lg2` (16 at the base size) are the two the audit asked for.
	t["font.sans"], t["font.mono"], t["font.display"] = a["font.sans"], a["font.mono"], a["font.display"]
	t["font.size.xxs"] = math.max(9, size - 3)
	t["font.size.xs"] = size - 2
	t["font.size.sm"] = size - 1
	t["font.size.md"] = size
	t["font.size.lg"] = size + 2
	t["font.size.lg2"] = size + 3
	t["font.size.xl"] = size + 7
	t["font.size.xxl"] = round(size * 2.15)
	t["font.line.xxs"] = math.max(11, line - 5)
	t["font.line.xs"] = line - 4
	t["font.line.sm"] = line - 2
	t["font.line.md"] = line
	t["font.line.lg"] = line + 2
	t["font.line.lg2"] = line + 4
	t["font.line.xl"] = line + 8
	t["font.line.xxl"] = line + 16
	-- The conversation's scale, from the same two numbers: prose a step above
	-- the body, its headings in real steps, its code at the body's size. This
	-- is what makes Settings' one control move the reading column too.
	t["zeron.prose.size"] = size + 2
	t["zeron.prose.line"] = round(size * 1.85)
	t["zeron.prose.h1.size"] = size + 9
	t["zeron.prose.h1.line"] = round((size + 9) * 1.35)
	t["zeron.prose.h2.size"] = size + 5
	t["zeron.prose.h2.line"] = round((size + 5) * 1.4)
	t["zeron.prose.h3.size"] = size + 2
	t["zeron.prose.h3.line"] = round(size * 1.85)
	t["zeron.prose.small.size"] = size + 1
	t["zeron.prose.small.line"] = line + 2
	t["zeron.prose.code.size"] = size
	t["zeron.prose.code.line"] = line + 2
	t["zeron.prose.code.inline"] = size + 0.5
	t["zeron.editor.size"] = size
	t["zeron.editor.line"] = line + 2
	t["font.weight.regular"] = (a["font.weight"] :: number) - 50
	t["font.weight.medium"] = a["font.weight"]
	t["font.weight.semibold"] = a["font.strong"]
	t["font.weight.bold"] = (a["font.strong"] :: number) + 100

	-- Space, corners, density, measure.
	t["space.1"] = unit
	t["space.2"] = unit * 2
	t["space.3"] = unit * 3
	t["space.4"] = unit * 4
	t["space.5"] = unit * 6
	t["space.6"] = unit * 8
	t["space.7"] = unit * 12
	t["space.8"] = unit * 16
	t["radius.xs"] = math.max(0, radius - 4)
	t["radius.sm"] = math.max(0, radius - 2)
	t["radius.md"] = radius
	t["radius.lg"] = radius + 2
	t["radius.xl"] = radius + 4
	t["radius.full"] = 9999
	t["control.height"] = a["metric.control"]
	t["density.row"] = a["metric.row"]
	t["density.row.compact"] = round((a["metric.row"] :: number) * 0.86)
	t["density.control"] = a["metric.control"]
	t["density.gutter"] = a["metric.gutter"]
	-- The band the title bar draws: one height everywhere (§KEPL-11 4.1),
	-- a control's plus its line.
	t["band.height"] = (a["metric.control"] :: number) + 4
	t["measure.conversation"] = a["metric.conversation"]
	t["measure.page"] = a["metric.page"]

	-- Motion, and the blur that is glass when it is on.
	t["motion.duration.fast"] = round(duration * 0.66)
	t["motion.duration.normal"] = duration
	t["motion.duration.slow"] = round(duration * 1.66)
	t["motion.ease.standard"] = a["metric.ease"]
	t["motion.ease.enter"] = "cubicBezier(0.16, 1, 0.3, 1)"
	t["motion.ease.exit"] = "cubicBezier(0.4, 0, 1, 1)"
	t["material.blur"] = a["metric.blur"]

	-- The app's own shapes, which a theme used to write one at a time.
	t["zeron.window"] = a["surface.window"]
	t["zeron.selected.bg"] = alpha(accent, 0.18)
	t["zeron.selected.ring"] = alpha(accent, 0.26)
	t["zeron.selected.flat.bg"] = alpha(accent, 0.15)
	t["zeron.menu.selected"] = alpha(accent, 0.16)
	t["zeron.composer.tint"] = a["surface.window"]
	t["zeron.composer.solid"] = a["surface.overlay"]
	t["zeron.composer.edge"] = alpha(accent, 0.24)
	t["zeron.primary.bg"] = a["ink.primary"]
	t["zeron.primary.text"] = a["surface.window"]
	t["zeron.destructive.bg"] = a["tone.danger"]
	t["zeron.destructive.text"] = a["accent.ink"]
	t["zeron.diff.added"], t["zeron.diff.removed"] = a["tone.ok"], a["tone.danger"]
	t["zeron.git.modified"], t["zeron.git.added"], t["zeron.git.deleted"] = a["tone.warn"], a["tone.ok"], a["tone.danger"]
	t["zeron.pr.open"], t["zeron.pr.merged"], t["zeron.pr.closed"] = a["tone.ok"], a["accent"], a["tone.danger"]
	t["zeron.status.working"] = a["ink.secondary"]
	t["zeron.status.input"] = a["accent"]
	t["zeron.status.approval"] = a["attention"]
	t["zeron.status.done"] = a["tone.ok"]
	t["zeron.status.failed"] = a["tone.danger"]
	t["zeron.status.limited"] = a["tone.warn"]
	t["zeron.status.dot.done"] = a["tone.ok"]
	t["zeron.status.dot.failed"] = a["tone.danger"]
	t["zeron.attention"] = a["attention"]
	t["kx.display"] = a["font.display"]
	t["kx.attention"] = a["attention"]

	-- Code's colours: the inks and tones, so a theme with none of its own
	-- still has readable ones. (`syntax.*` is the one group a theme may add
	-- beyond the 42, for hues of its own.)
	t["zeron.syntax.text"] = alpha(ink, 0.90)
	t["zeron.syntax.keyword"] = a["accent"]
	t["zeron.syntax.string"] = a["tone.ok"]
	t["zeron.syntax.escape"] = a["tone.warn"]
	t["zeron.syntax.regex"] = a["tone.info"]
	t["zeron.syntax.number"] = a["tone.warn"]
	t["zeron.syntax.constant"] = a["tone.warn"]
	t["zeron.syntax.comment"] = alpha(ink, 0.46)
	t["zeron.syntax.function"] = a["tone.info"]
	t["zeron.syntax.macro"] = a["accent"]
	t["zeron.syntax.type"] = a["tone.warn"]
	t["zeron.syntax.variable"] = a["tone.danger"]
	t["zeron.syntax.property"] = a["ink.primary"]
	t["zeron.syntax.attribute"] = a["attention"]
	t["zeron.syntax.label"] = a["tone.danger"]
	t["zeron.syntax.operator"] = alpha(ink, 0.66)
	t["zeron.syntax.punctuation"] = alpha(ink, 0.50)
	t["zeron.syntax.tag"] = a["tone.danger"]
	t["zeron.syntax.heading"] = a["accent"]
	t["zeron.syntax.link"] = a["tone.info"]
	t["zeron.syntax.url"] = alpha(ink, 0.42)
	t["zeron.syntax.emphasis"] = alpha(ink, 0.90)
	t["zeron.syntax.strong"] = a["ink.primary"]
	t["zeron.syntax.quote"] = alpha(ink, 0.60)
	t["zeron.syntax.inserted"] = a["tone.ok"]
	t["zeron.syntax.deleted"] = a["tone.danger"]
	t["zeron.syntax.hunk"] = a["accent"]
	t["zeron.syntax.meta"] = alpha(ink, 0.60)

	return t
end

-- A flat table of dotted names as the nested table `ui.theme.register`
-- takes. A value that is also a group's head (`accent`) has its children
-- beside it: the window registers `accent` and `accent.ink` as two tokens.
local function nest(flat: { [string]: any }): { [string]: any }
	local root: { [string]: any } = {}
	local names: { string } = {}
	for name in pairs(flat) do
		table.insert(names, name)
	end
	table.sort(names)
	for _, name in names do
		local parts = string.split(name, ".")
		if #parts == 1 then
			root[name] = flat[name]
		else
			local at = root
			local ok = true
			for i = 1, #parts - 1 do
				local key = parts[i] :: string
				local next_at = at[key]
				if next_at == nil then
					next_at = {}
					at[key] = next_at
				elseif type(next_at) ~= "table" then
					-- (A token and a group of the same name: the group's
					-- children cannot live beside it in a nested table, so
					-- they are named flat, which the reader takes too.)
					ok = false
					break
				end
				at = next_at :: any
			end
			if ok then
				at[parts[#parts] :: string] = flat[name]
			else
				root[name] = flat[name]
			end
		end
	end
	return root
end

-- The checked reports, one per scheme (`{ dark = Report, light = Report }`),
-- as src/authored.rs hands them over. A scheme whose theme did not check is
-- nil here and is never worn.
function M.reports(): { [string]: any }
	local ok, shared = pcall(ui.shared, "themes.authored", nil :: any)
	if not ok or shared == nil then
		return {}
	end
	local now = shared:get()
	return if type(now) == "table" then now else {}
end

M.register = function(values: { [string]: any }, scheme: string, name: string?)
	local ok, err = pcall(ui.theme.register, {
		name = name or (if scheme == "light" then "zeron-light" else "zeron-dark"),
		type = if scheme == "light" then "light" else "dark",
		extends = if scheme == "light" then "zgx-light" else "zgx-dark",
		tokens = nest(M.derive(values, scheme)),
	} :: any)
	if not ok then
		print(`authored: the {scheme} theme did not register: {tostring(err)}`)
	end
end

-- The 42 as the shipped file has them, for a host with no window to check
-- them (a fixture, a plugin lent the design system): the file is loaded, worn
-- for a moment, and its authored values read back. Nil where there is no
-- such file.
local function from_file(scheme: string): ({ [string]: any }?)

	local path = if scheme == "light" then "ds/themes/keplorayx-light.json" else "ds/themes/keplorayx-dark.json"
	local ok, name = pcall(ui.theme.load, path)
	if not ok or type(name) ~= "string" then
		return nil
	end
	-- (Worn for the length of the read, then put back: a host that reads this
	-- is not asking to change which theme the window wears.)
	local was = ui.theme.current:peek()
	pcall(function()
		ui.theme.current:set(name)
	end)
	local values: { [string]: any } = {}
	for _, group in ipairs(M.SPEC) do
		for _, token_name in ipairs(group.names) do
			local v = ui.theme.get(token_name)
			if v == nil then
				values = nil
				break
			end
			values[token_name] = v
		end
		if values == nil then
			break
		end
	end
	if type(was) == "string" and was ~= name then
		pcall(function()
			ui.theme.current:set(was)
		end)
	end
	return values
end

-- The window's two themes, from what the check said. What did not check is
-- printed with the token it named, and that scheme wears the framework's
-- own rather than a theme nobody could check.
function M.wear(size: number?): { dark: boolean, light: boolean }
	local worn = { dark = false, light = false }
	for _, scheme in ipairs({ "dark", "light" }) do
		local report = M.reports()[scheme]
		if type(report) == "table" and report.ok and type(report.tokens) == "table" then
			local values = table.clone(report.tokens)
			if size then
				values["font.size"] = size
				values["font.line"] = math.max(values["font.line"] :: number, math.ceil(size * 1.2))
			end
			M.register(values, scheme)
			worn[scheme :: any] = true
			for _, line in ipairs(report.warnings or {}) do
				print(`theme: {scheme}: {line.message}`)
			end
		elseif type(report) == "table" and not report.ok then
			for _, line in ipairs(report.errors or {}) do
				print(`theme: {scheme}: stage {line.stage} ({(M.STAGES or {})[line.stage] or "check"}): {line.message}`)
			end
		else
			-- (No window checked these: a fixture, or a plugin lent the design
			-- system. The shipped file is worn and read back, so the same
			-- derivation runs over the same values.)
			local values = from_file(scheme)
			if values then
				if size then
					values["font.size"] = size
					values["font.line"] = math.max(values["font.line"] :: number, math.ceil(size * 1.2))
				end
				M.register(values, scheme)
				worn[scheme :: any] = true
			end
		end
	end
	return worn
end

M.STAGES = {
	"the file is JSON",
	"its shape",
	"every key is authored, of its kind, in range",
	"the references resolve",
	"the inks read",
	"the layout holds",
	"the lint",
}

return M
