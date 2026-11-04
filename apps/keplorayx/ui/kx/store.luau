--!strict
-- The engine as the screens see it (src/host.rs): shared state and calls,
-- in one place, so a fixture can stand in for the host. With a fixture
-- asked for (`KX_FIXTURE=<name>`, ui/fixtures/<name>.luau), the states are
-- the fixture's and the calls are its stubs: every screen runs headless.

export type Thread = {
	id: string,
	title: string,
	project: string,
	project_id: string,
	status: string,
	harness: string,
	harness_id: string,
	updated: number,
	unread: boolean,
}

export type Project = { id: string, name: string, path: string }

export type Change = { path: string, added: number, removed: number }

export type Tool = { name: string, state: string, output: string, changes: { Change } }

export type Entry = { id: string, kind: string, text: string, tool: Tool? }

export type Draft = { id: string, kind: string, text: string }

export type Open = { id: string, title: string, harness: string, running: boolean, status: string }

export type Question = { text: string, options: { string } }

export type Approval = { request_id: string, tool: string, summary: string, questions: { Question } }

export type Agent = { id: string, label: string }

local fixture_name = (ui.shared("dev.fixture", "") :: any):peek() :: string

local store = {}

store.fixture = fixture_name

if fixture_name ~= "" then
	local ok, data = pcall(require, "@fixtures/" .. fixture_name)
	local f: any = if ok then data else {}
	store.threads = ui.state("fx.threads", (f.threads or {}) :: { Thread })
	store.projects = ui.state("fx.projects", (f.projects or {}) :: { Project })
	store.thread = ui.state("fx.thread", f.thread :: Open?)
	store.entries = ui.state("fx.entries", (f.entries or {}) :: { Entry })
	store.draft = ui.state("fx.draft", (f.draft or { id = "", kind = "", text = "" }) :: Draft)
	store.approval = ui.state("fx.approval", f.approval :: Approval?)
	store.agents = ui.state("fx.agents", (f.agents or {}) :: { Agent })
	store.route = f.route
else
	store.threads = ui.shared("kx.threads", {} :: { Thread })
	store.projects = ui.shared("kx.projects", {} :: { Project })
	store.thread = ui.shared("kx.thread", nil :: Open?)
	store.entries = ui.shared("kx.entries", {} :: { Entry })
	store.draft = ui.shared("kx.draft", { id = "", kind = "", text = "" } :: Draft)
	store.approval = ui.shared("kx.approval", nil :: Approval?)
	store.agents = ui.shared("kx.agents", {} :: { Agent })
end

local function stubbed(): boolean
	return fixture_name ~= ""
end

function store.open(id: string)
	if stubbed() then
		for _, t in store.threads:peek() do
			if t.id == id then
				store.thread:set({ id = t.id, title = t.title, harness = t.harness, running = false, status = "" })
			end
		end
		return
	end
	ui.invoke("kx.open", id)
end

function store.home()
	if stubbed() then
		store.thread:set(nil)
		return
	end
	ui.invoke("kx.new")
end

function store.interrupt()
	if not stubbed() then
		ui.invoke("kx.interrupt")
	end
end

function store.decide(request_id: string, decision: string, answers: { { string } }?)
	if stubbed() then
		store.approval:set(nil)
		return
	end
	ui.invoke("kx.decide", { request_id = request_id, decision = decision, answers = answers })
end

function store.send(text: string, project: string?): any
	if stubbed() then
		return ui.sleep(0)
	end
	return ui.call("kx.send", { text = text, project = project })
end

function store.add_project(path: string): any
	if stubbed() then
		return ui.sleep(0)
	end
	return ui.call("kx.add_project", path)
end

-- A thread is the open one.
function store.is_open(id: string): boolean
	local t = store.thread:get()
	return t ~= nil and t.id == id
end

return store
