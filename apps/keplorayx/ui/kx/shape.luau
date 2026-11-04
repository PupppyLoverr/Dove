--!strict
-- Concentric corners. Anything inside a rounded container (its hover
-- wash and selection included) takes the container's corner less the
-- inset between them, so the two curves run parallel; an inset at or
-- past the corner is square, and a pill's contents are pills. Never a
-- fixed radius inside a rounded parent.
--
-- The arithmetic and `contain` are ported from Zeron's zgx app
-- (ui/ds/control.luau, MIT; see THIRD_PARTY_NOTICES.md).
--
--   shape.nest("$radius.lg", 1, "$space.1")   -> the inner corner, points
--   shape.contain(props, { radius = "$radius.lg", inset = "$space.1" })

local PILL = 9999

local M = {}

M.PILL = PILL

-- A radius or inset in points: a number, or a token ("$radius.lg").
function M.points(v: any): number
	if type(v) == "number" then
		return v
	elseif v == "$radius.full" then
		return PILL
	elseif type(v) == "string" and string.sub(v, 1, 1) == "$" then
		return (ui.theme.get(string.sub(v, 2)) :: number?) or 0
	end
	return 0
end

-- The corner `...` in from the edge of a corner `outer` (a border, then
-- padding, as many as stand between).
function M.nest(outer: any, ...: any): number
	local r = M.points(outer)
	if r >= PILL then
		return PILL
	end
	for _, inset in { ... } do
		r -= M.points(inset)
	end
	return math.max(0, r)
end

-- A box that holds controls: its corner, one even inset on all four sides
-- (a `border` adds to it), and every radius token inside rescoped to the
-- nested corner, so a Button in it comes out concentric in all its states
-- without being told. Adds them to `props`, and returns it.
function M.contain(props: { [any]: any }, box: { radius: any, inset: any, border: number? }): { [any]: any }
	local inner = M.nest(box.radius, box.border or 0, box.inset)
	props.rounded = box.radius
	props.p = box.inset
	local scope: { [string]: any } = {
		radius = { sm = math.min(inner, M.points("$radius.sm")), md = inner, lg = inner, xl = inner },
	}
	props.theme = scope
	return props
end

return M
