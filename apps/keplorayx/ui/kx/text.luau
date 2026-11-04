--!strict
-- Type does the work: hierarchy from weight and space, never boxes. One
-- function per role, so a screen says what a line is, not how it looks.
--
--   Title { "Good evening" }        the display face, a screen's one title
--   Heading { "Projects" }          a section's
--   Body { text }                   reading text, generous lines
--   Label { "Model" }               a control's, medium weight
--   Caption { "2 min ago" }         secondary, small
--   Mono { "cargo test" }           agent output and code
--   Numeric { cost }                figures that change in place: tabular

local M = {}

type P = { [any]: any }

local function with(props: P, base: P): Node
	local p = table.clone(props)
	for k, v in base do
		if p[k] == nil then
			p[k] = v
		end
	end
	return ui.text(p)
end

-- The display face (Instrument Serif) has one weight: a title is set large
-- and regular, never bolded.
function M.Title(props: P): Node
	return with(props, {
		font = "$kx.display",
		text_size = "$font.size.xxl",
		line_height = "$font.line.xxl",
		weight = "$font.weight.regular",
		letter_spacing = -0.2,
		color = "$text.primary",
		heading = 1,
	})
end

function M.Heading(props: P): Node
	return with(props, {
		font = "$font.sans",
		text_size = "$font.size.lg",
		line_height = "$font.line.lg",
		weight = "$font.weight.medium",
		letter_spacing = -0.1,
		color = "$text.primary",
		heading = 2,
	})
end

function M.Body(props: P): Node
	return with(props, {
		text_size = "$font.size.md",
		line_height = "$font.line.md",
		color = "$text.primary",
		wrap = true,
	})
end

function M.Label(props: P): Node
	return with(props, {
		text_size = "$font.size.md",
		line_height = "$font.line.md",
		weight = "$font.weight.medium",
		color = "$text.primary",
	})
end

function M.Caption(props: P): Node
	return with(props, {
		text_size = "$font.size.sm",
		line_height = "$font.line.sm",
		color = "$text.secondary",
	})
end

-- The quietest line: metadata, timestamps.
function M.Hint(props: P): Node
	return with(props, {
		text_size = "$font.size.xs",
		line_height = "$font.line.xs",
		color = "$text.tertiary",
	})
end

function M.Mono(props: P): Node
	return with(props, {
		font = "$font.mono",
		text_size = "$font.size.sm",
		line_height = "$font.line.sm",
		color = "$text.primary",
	})
end

function M.Numeric(props: P): Node
	return with(props, {
		font = "$kx.numeric",
		tabular = true,
		text_size = "$font.size.sm",
		line_height = "$font.line.sm",
		color = "$text.secondary",
	})
end

return M
