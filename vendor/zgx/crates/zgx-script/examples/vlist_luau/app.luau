-- The variable-height list comparison scene (zgx_workload::vlist) in Luau:
-- 100,000 chat rows of wrapped text, each as tall as its text and image
-- make it, in a measured list. The host (examples/vlist_luau.rs) publishes
-- the scene's sizes and colours, gives each row's text, and scrolls it.
type Scene = {
	rows: number,
	estimate: number,
	font: string,
	font_size: number,
	line_height: number,
	pad_x: number,
	pad_y: number,
	gap: number,
	text_width: number,
	image_w: number,
	image_h: number,
	background: string,
	row_colors: { string },
	author: string,
	text: string,
	muted: string,
	image: string,
	probe: boolean,
}
type Row = { author: string, time: string, text: string, image: boolean }

local scene: Scene = ui.shared("scene", {} :: Scene):peek()
local offset = ui.shared("offset", 0)
-- The host's jumps: { how many so far, the row (from 0) }.
local jump = ui.shared("jump", { 0, 0 })
local list = ui.ref()

ui.effect(function()
	local to = jump:get()
	local el = list:peek()
	if to[1] > 0 and el then
		el:scroll_to_row(to[2] + 1, { align = "start" })
	end
end)

-- The probe compares each row's laid-out height across frameworks.
local function report(i: number)
	return function(size)
		ui.call_sync("height", { i - 1, size.h })
	end
end

local function message(i: number)
	local row: Row = ui.call_sync("row", i - 1)
	return ui.column {
		w_full = true,
		px = scene.pad_x,
		py = scene.pad_y,
		gap = scene.gap,
		bg = scene.row_colors[(i - 1) % 2 + 1],
		on_resize = if scene.probe then report(i) else nil,
		ui.row {
			gap = 8,
			h = scene.line_height,
			ui.text { row.author, literal = true, color = scene.author },
			ui.text { row.time, literal = true, color = scene.muted },
		},
		ui.text { row.text, literal = true, wrap = true, w = scene.text_width },
		if row.image then ui.div { w = scene.image_w, h = scene.image_h, bg = scene.image } else nil,
	}
end

return ui.column {
	w_full = true,
	h_full = true,
	bg = scene.background,
	color = scene.text,
	font = scene.font,
	text_size = scene.font_size,
	line_height = scene.line_height,
	ui.list {
		count = scene.rows,
		measured = true,
		height = scene.estimate,
		offset = offset,
		ref = list,
		w_full = true,
		h_full = true,
		row = message,
	},
}
