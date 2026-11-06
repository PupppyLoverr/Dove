--!strict
-- Sums and units, worked out as you type (the launcher's first row). A
-- small parser of its own, never `load`: numbers, + - * / % ^, brackets,
-- a few functions (sqrt, abs, round, floor, ceil, sin, cos, tan, log, ln,
-- exp) and constants (pi, e). And conversions: `5 km to mi`, `72 f in c`,
-- `3.5 gb to mb`, `90 min to h`.
--
--   calc.eval("2 * (3 + 4)")  → 14 (nil when it isn't a sum)
--   calc.convert("5 km to mi") → 3.10686, "mi" (nil when it isn't one)
--   calc.format(14.000001)    → "14"

local M = {}

local FUNCS: { [string]: (number) -> number } = {
	sqrt = math.sqrt,
	abs = math.abs,
	round = function(x: number): number
		return math.floor(x + 0.5)
	end,
	floor = math.floor,
	ceil = math.ceil,
	sin = math.sin,
	cos = math.cos,
	tan = math.tan,
	log = function(x: number): number
		return math.log10(x)
	end,
	ln = function(x: number): number
		return math.log(x)
	end,
	exp = math.exp,
}
local CONSTS: { [string]: number } = { pi = math.pi, e = math.exp(1) }

type Token = { kind: string, value: any }

local function tokens(s: string): { Token }?
	local out: { Token } = {}
	local i = 1
	while i <= #s do
		local c = string.sub(s, i, i)
		if string.match(c, "%s") then
			i += 1
		elseif string.match(c, "[%d%.]") then
			local num = string.match(s, "^%d*%.?%d+", i) or string.match(s, "^%d+%.?", i)
			if not num then
				return nil
			end
			local n = tonumber(num)
			if not n then
				return nil
			end
			table.insert(out, { kind = "num", value = n })
			i += #num
		elseif string.match(c, "%a") then
			local word = string.match(s, "^%a+", i) :: string
			table.insert(out, { kind = "word", value = string.lower(word) })
			i += #word
		elseif string.find("+-*/%^()×÷", c, 1, true) then
			local op = if c == "×" then "*" elseif c == "÷" then "/" else c
			table.insert(out, { kind = "op", value = op })
			i += 1
		elseif c == "," then
			i += 1
		else
			return nil
		end
	end
	return out
end

-- Recursive descent over the tokens; an error is a `nil` answer.
local function parse(list: { Token }): number?
	local at = 1
	local function peek(): Token?
		return list[at]
	end
	local function take(): Token?
		at += 1
		return list[at - 1]
	end
	local expr: () -> number
	local function atom(): number
		local t = take()
		if not t then
			error("end")
		end
		if t.kind == "num" then
			return t.value
		elseif t.kind == "op" and t.value == "(" then
			local v = expr()
			local close = take()
			if not close or close.value ~= ")" then
				error("bracket")
			end
			return v
		elseif t.kind == "op" and t.value == "-" then
			return -atom()
		elseif t.kind == "op" and t.value == "+" then
			return atom()
		elseif t.kind == "word" then
			if CONSTS[t.value] then
				return CONSTS[t.value]
			end
			local f = FUNCS[t.value]
			if f then
				return f(atom())
			end
		end
		error("token")
	end
	local function power(): number
		local base = atom()
		local t = peek()
		if t and t.kind == "op" and t.value == "^" then
			take()
			return base ^ power()
		end
		return base
	end
	local function term(): number
		local v = power()
		while true do
			local t = peek()
			if t and t.kind == "op" and (t.value == "*" or t.value == "/" or t.value == "%") then
				take()
				local r = power()
				v = if t.value == "*" then v * r elseif t.value == "/" then v / r else v % r
			elseif t and (t.kind == "num" or (t.kind == "op" and t.value == "(")) then
				-- (2(3) and 2pi: side by side, multiplied.)
				v *= power()
			else
				return v
			end
		end
	end
	function expr(): number
		local v = term()
		while true do
			local t = peek()
			if t and t.kind == "op" and (t.value == "+" or t.value == "-") then
				take()
				local r = term()
				v = if t.value == "+" then v + r else v - r
			else
				return v
			end
		end
	end
	local ok, v = pcall(expr)
	if not ok or at <= #list or type(v) ~= "number" or v ~= v or v == math.huge or v == -math.huge then
		return nil
	end
	return v
end

-- A sum's answer: only for what reads as one (an operator or a function in
-- it, not a lone number or a word).
function M.eval(s: string): number?
	local list = tokens(s)
	if not list or #list < 2 then
		return nil
	end
	local sum = false
	for _, t in list do
		if t.kind == "op" and t.value ~= "(" and t.value ~= ")" then
			sum = true
		elseif t.kind == "word" then
			if not (FUNCS[t.value] or CONSTS[t.value]) then
				return nil
			end
			sum = true
		end
	end
	if not sum then
		return nil
	end
	return parse(list)
end

-- Units: each in its kind's base (metres, grams, bytes, seconds).
local UNITS: { [string]: { kind: string, f: number } } = {
	mm = { kind = "length", f = 0.001 },
	cm = { kind = "length", f = 0.01 },
	m = { kind = "length", f = 1 },
	km = { kind = "length", f = 1000 },
	["in"] = { kind = "length", f = 0.0254 },
	inch = { kind = "length", f = 0.0254 },
	ft = { kind = "length", f = 0.3048 },
	yd = { kind = "length", f = 0.9144 },
	mi = { kind = "length", f = 1609.344 },
	mg = { kind = "mass", f = 0.001 },
	g = { kind = "mass", f = 1 },
	kg = { kind = "mass", f = 1000 },
	lb = { kind = "mass", f = 453.59237 },
	oz = { kind = "mass", f = 28.349523125 },
	b = { kind = "data", f = 1 },
	kb = { kind = "data", f = 1024 },
	mb = { kind = "data", f = 1024 ^ 2 },
	gb = { kind = "data", f = 1024 ^ 3 },
	tb = { kind = "data", f = 1024 ^ 4 },
	ms = { kind = "time", f = 0.001 },
	s = { kind = "time", f = 1 },
	sec = { kind = "time", f = 1 },
	min = { kind = "time", f = 60 },
	h = { kind = "time", f = 3600 },
	hr = { kind = "time", f = 3600 },
	day = { kind = "time", f = 86400 },
	days = { kind = "time", f = 86400 },
	week = { kind = "time", f = 604800 },
	weeks = { kind = "time", f = 604800 },
	ml = { kind = "volume", f = 0.001 },
	l = { kind = "volume", f = 1 },
	gal = { kind = "volume", f = 3.785411784 },
	c = { kind = "temp", f = 0 },
	f = { kind = "temp", f = 0 },
	k = { kind = "temp", f = 0 },
}

local function to_kelvin(v: number, u: string): number
	return if u == "c" then v + 273.15 elseif u == "f" then (v - 32) * 5 / 9 + 273.15 else v
end
local function from_kelvin(v: number, u: string): number
	return if u == "c" then v - 273.15 elseif u == "f" then (v - 273.15) * 9 / 5 + 32 else v
end

function M.convert(s: string): (number?, string?)
	local n, a, b = string.match(string.lower(s), "^%s*(%-?[%d%.]+)%s*(%a+)%s+[ti][on]%s+(%a+)%s*$")
	if not n then
		return nil, nil
	end
	local v = tonumber(n)
	local from, to = UNITS[a :: string], UNITS[b :: string]
	if not v or not from or not to or from.kind ~= to.kind then
		return nil, nil
	end
	if from.kind == "temp" then
		return from_kelvin(to_kelvin(v, a :: string), b :: string), b
	end
	return v * from.f / to.f, b
end

function M.format(v: number): string
	if math.abs(v - math.floor(v + 0.5)) < 1e-9 and math.abs(v) < 1e15 then
		return string.format("%d", math.floor(v + 0.5))
	end
	local s = string.format(if math.abs(v) >= 1 then "%.4f" else "%.6f", v)
	s = string.gsub(s, "0+$", "")
	s = string.gsub(s, "%.$", "")
	return s
end

return M
