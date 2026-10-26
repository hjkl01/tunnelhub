import { useCallback, useEffect, useMemo, useState, type FormEvent, type ReactNode } from 'react'
import { QRCodeSVG } from 'qrcode.react'
import { ArrowUpRight, CircleHelp, Globe2, LayoutDashboard, LogOut, Maximize2, Minimize2, Network, Plus, RefreshCw, ScrollText, Search, Settings2, ShieldCheck, Trash2, Waypoints, X } from 'lucide-react'

type Service = {
  id: string
  name: string
  target_url: string
  enabled: boolean
  created_at: string
  updated_at: string
  access_mode: string
}
type Draft = { name: string; target_url: string; enabled: boolean }
type TargetTestResult = { reachable: boolean; status: number; final_url: string; content_type?: string | null }
const emptyDraft: Draft = { name: '', target_url: 'http://host.docker.internal:', enabled: true }

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { 'Content-Type': 'application/json', ...init?.headers },
  })
  if (!response.ok) {
    if (response.status === 401) {
      window.location.reload()
    }
    const body = await response.json().catch(() => null)
    throw new Error(body?.error ?? `请求失败（${response.status}）`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}


type AccessUser = { id: number; username: string; enabled: boolean }
type AccessGroup = { id: number; name: string; members: number[] }
type ServiceAcl = { access_mode: 'public' | 'restricted'; users: number[]; groups: number[] }

function AccessControlPanel({ services }: { services: Service[] }) {
  const [users, setUsers] = useState<AccessUser[]>([])
  const [groups, setGroups] = useState<AccessGroup[]>([])
  const [acls, setAcls] = useState<Record<string, ServiceAcl>>({})
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [groupName, setGroupName] = useState('')
  const [message, setMessage] = useState('')
  const load = useCallback(async () => {
    const u = await api<AccessUser[]>('/api/services/access/users')
    const g = await api<AccessGroup[]>('/api/services/access/groups')
    setUsers(u); setGroups(g)
    const entries = await Promise.all(services.map(async s => [s.id, await api<ServiceAcl>('/api/services/' + s.id + '/acl')] as const))
    setAcls(Object.fromEntries(entries))
  }, [services])
  useEffect(() => { void load().catch(e => setMessage(String(e))) }, [load])
  return <section className="page-content">
    <div className="page-heading"><div><div className="eyebrow">IDENTITY & ACCESS</div><h1>用户与权限</h1><p>创建用户和用户组，为每个服务配置访问授权。</p></div><button className="secondary-button" onClick={() => void load()}>刷新</button></div>
    {message && <div className="alert error-alert">{message}</div>}
    <section className="panel access-panel"><div className="panel-heading"><div><h2>用户账户</h2><p>业务服务登录用户，密码至少 12 位</p></div></div>
      <form className="inline-create" onSubmit={async e => { e.preventDefault(); try { await api('/api/services/access/users', { method: 'POST', body: JSON.stringify({ username, password }) }); setUsername(''); setPassword(''); setMessage('用户已创建'); await load() } catch (err) { setMessage(String(err)) } }}>
        <input required minLength={3} value={username} onChange={e => setUsername(e.target.value)} placeholder="用户名" />
        <input required minLength={12} type="password" value={password} onChange={e => setPassword(e.target.value)} placeholder="初始密码" />
        <button className="primary-button">创建用户</button>
      </form>
      {users.map(u => <div className="access-row" key={u.id}><strong>{u.username}</strong><span>{u.enabled ? '已启用' : '已禁用'}</span><div><button className="secondary-button" onClick={async () => { const nextPassword = window.prompt('输入新密码（至少 12 位）'); if (nextPassword && nextPassword.length >= 12) { await api('/api/services/access/users/' + u.id, { method: 'PUT', body: JSON.stringify({ password: nextPassword }) }); setMessage('密码已重置'); await load() } }}>{'重置密码'}</button><button className="secondary-button" onClick={async () => { await api('/api/services/access/users/' + u.id, { method: 'PUT', body: JSON.stringify({ enabled: !u.enabled }) }); await load() }}>{u.enabled ? '禁用' : '启用'}</button><button className="secondary-button" onClick={async () => { if (window.confirm('确定删除用户 ' + u.username + '？')) { await api('/api/services/access/users/' + u.id, { method: 'DELETE' }); await load() } }}>删除</button></div></div>)}
    </section>
    <section className="panel access-panel"><div className="panel-heading"><div><h2>用户组</h2><p>用户组可以被授予服务访问权限</p></div></div>
      <form className="inline-create" onSubmit={async e => { e.preventDefault(); await api('/api/services/access/groups', { method: 'POST', body: JSON.stringify({ name: groupName }) }); setGroupName(''); await load() }}><input required value={groupName} onChange={e => setGroupName(e.target.value)} placeholder="用户组名称" /><button className="primary-button">新建组</button></form>
      {groups.map(g => <details className="group-item" key={g.id}><summary>{g.name}（{g.members.length} 人）</summary>{users.map(u => <label key={u.id}><input type="checkbox" checked={g.members.includes(u.id)} onChange={e => setGroups(old => old.map(x => x.id === g.id ? { ...x, members: e.target.checked ? [...x.members, u.id] : x.members.filter(id => id !== u.id) } : x))} />{u.username}</label>)}<button className="secondary-button" onClick={async () => { await api('/api/services/access/groups/' + g.id, { method: 'PUT', body: JSON.stringify({ members: g.members }) }); await load() }}>保存成员</button></details>)}
    </section>
    <section className="panel service-acl-panel"><div className="panel-heading"><div><h2>服务访问授权</h2><p>受限模式只允许指定用户和用户组访问。</p></div></div>
      {services.map(s => { const a = acls[s.id] || { access_mode: s.access_mode || 'public', users: [], groups: [] }; return <article className="service-acl-card" key={s.id}><div className="service-acl-title"><strong>{s.name}</strong><select value={a.access_mode} onChange={e => setAcls(old => ({ ...old, [s.id]: { ...a, access_mode: e.target.value as ServiceAcl['access_mode'] } }))}><option value="public">公开</option><option value="restricted">仅授权用户/组</option></select></div>{a.access_mode === 'restricted' && <div className="acl-subjects"><div><strong>用户</strong>{users.map(u => <label key={u.id}><input type="checkbox" checked={a.users.includes(u.id)} onChange={e => setAcls(old => ({ ...old, [s.id]: { ...a, users: e.target.checked ? [...a.users, u.id] : a.users.filter(id => id !== u.id) } }))} />{u.username}</label>)}</div><div><strong>用户组</strong>{groups.map(g => <label key={g.id}><input type="checkbox" checked={a.groups.includes(g.id)} onChange={e => setAcls(old => ({ ...old, [s.id]: { ...a, groups: e.target.checked ? [...a.groups, g.id] : a.groups.filter(id => id !== g.id) } }))} />{g.name}</label>)}</div></div>}<button className="primary-button" onClick={async () => { await api('/api/services/' + s.id + '/acl', { method: 'PUT', body: JSON.stringify(a) }); await load(); setMessage('授权已保存') }}>保存授权</button></article> })}
    </section>
  </section>
}

type NetworkInfo = {
  version: string
  listen: string
  caddy_enabled: boolean
  caddy_listen: string | null
  caddy_config_path: string | null
  caddy_config_exists: boolean
  ui_host: string | null
  service_domain: string | null
  cookie_secure: boolean
  database_url: string | null
  total_services: number
  enabled_services: number
  caddyfile: string | null
}

type AccessLog = {
  id: number
  service_id: string
  service_name: string | null
  method: string | null
  path: string | null
  client_ip: string | null
  user_agent: string | null
  username: string | null
  status: number
  created_at: string
}

function ConfigRow({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return <div className="config-row"><span>{label}</span><div><code>{value}</code>{hint && <small>{hint}</small>}</div></div>
}

function logStatus(status: number) {
  if (status === 200) return { label: '已放行', cls: 'enabled' }
  if (status === 302) return { label: '需登录', cls: 'pending' }
  if (status === 403) return { label: '无权限', cls: 'denied' }
  if (status === 404) return { label: '服务不存在', cls: 'disabled' }
  return { label: `HTTP ${status}`, cls: 'disabled' }
}

function NetworkPage() {
  const [info, setInfo] = useState<NetworkInfo | null>(null)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)
  const load = useCallback(async () => {
    setLoading(true)
    try {
      setInfo(await api<NetworkInfo>('/api/services/network'))
      setError('')
    } catch (e) {
      setError(e instanceof Error ? e.message : '无法加载网络组件信息')
    } finally {
      setLoading(false)
    }
  }, [])
  useEffect(() => { void load() }, [load])
  return <section className="page-content">
    <div className="page-heading"><div><div className="eyebrow">NETWORK COMPONENTS</div><h1>网络组件</h1><p>查看内置反向代理、监听地址与服务域名等运行时配置。</p></div><button className="secondary-button" onClick={() => void load()}><RefreshCw size={16} className={loading ? 'spin' : ''} />刷新</button></div>
    {error && <div className="alert error-alert"><span>{error}</span></div>}
    {info && <>
      <div className="stats-grid">
        <div className="stat-card"><div className="stat-label">反向代理</div><div className="stat-value">{info.caddy_enabled ? 'Caddy' : '未启用'}</div><div className="stat-foot"><span className={`mini-dot ${info.caddy_enabled ? 'green' : ''}`} />{info.caddy_enabled ? '内置 Caddy 正在转发业务流量' : '仅通过同源路径代理，未启动 Caddy'}</div></div>
        <div className="stat-card"><div className="stat-label">API 监听地址</div><div className="stat-value net-value">{info.listen}</div><div className="stat-foot">TunnelHub 后端监听地址</div></div>
        <div className="stat-card"><div className="stat-label">已启用服务</div><div className="stat-value">{info.enabled_services}<span>/ {info.total_services} 个</span></div><div className="stat-foot">已同步到代理引擎的路由</div></div>
      </div>
      <section className="panel network-panel">
        <div className="panel-heading"><div><h2>运行时配置</h2><p>来自当前进程的环境变量</p></div></div>
        <div className="config-list">
          <ConfigRow label="Caddy 监听地址" value={info.caddy_listen ?? '未设置'} />
          <ConfigRow label="Caddy 配置文件" value={info.caddy_config_path ?? '未设置'} hint={info.caddy_config_exists ? '已生成' : '不存在'} />
          <ConfigRow label="管理域名（TUNNELHUB_UI_HOST）" value={info.ui_host ?? '任意域名均可访问'} />
          <ConfigRow label="服务域名（TUNNELHUB_SERVICE_DOMAIN）" value={info.service_domain ?? '未启用子域名路由'} />
          <ConfigRow label="Cookie Secure" value={info.cookie_secure ? '开启' : '关闭'} />
          <ConfigRow label="数据库" value={info.database_url ?? '默认本地 SQLite'} />
          <ConfigRow label="版本" value={info.version} />
        </div>
      </section>
      {info.caddyfile && <section className="panel network-panel"><div className="panel-heading"><div><h2>生成的 Caddyfile</h2><p>由 TunnelHub 根据已启用服务自动维护</p></div></div><pre className="code-block">{info.caddyfile}</pre></section>}
    </>}
  </section>
}

function LogsPage() {
  const [logs, setLogs] = useState<AccessLog[]>([])
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(true)
  const [auto, setAuto] = useState(true)
  const [query, setQuery] = useState('')
  const load = useCallback(async () => {
    try {
      setLogs(await api<AccessLog[]>('/api/services/logs?limit=200'))
      setError('')
    } catch (e) {
      setError(e instanceof Error ? e.message : '无法加载访问日志')
    } finally {
      setLoading(false)
    }
  }, [])
  useEffect(() => { void load() }, [load])
  useEffect(() => {
    if (!auto) return
    const timer = window.setInterval(() => { void load() }, 5000)
    return () => window.clearInterval(timer)
  }, [auto, load])
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return logs
    return logs.filter(l => [l.service_name, l.service_id, l.username, l.path, l.client_ip, l.user_agent, l.method].some(value => (value ?? '').toLowerCase().includes(needle)))
  }, [logs, query])
  return <section className="page-content">
    <div className="page-heading"><div><div className="eyebrow">ACCESS LOGS</div><h1>访问日志</h1><p>记录通过代理访问业务服务的鉴权结果，仅保留最近 5000 条。</p></div><button className="secondary-button" onClick={() => void load()}><RefreshCw size={16} className={loading ? 'spin' : ''} />刷新</button></div>
    {error && <div className="alert error-alert"><span>{error}</span></div>}
    <section className="panel">
      <div className="panel-heading"><div><h2>最近访问</h2><p>放行 / 需登录 / 无权限 / 服务不存在</p></div><label className="auto-toggle"><input type="checkbox" checked={auto} onChange={e => setAuto(e.target.checked)} />每 5 秒自动刷新</label></div>
      <div className="logs-toolbar"><div className="search-box"><Search size={16} /><input value={query} onChange={e => setQuery(e.target.value)} placeholder="搜索服务、用户、路径或来源 IP" /></div><span className="result-count">{filtered.length} 条</span></div>
      {loading ? <div className="empty-state">正在加载访问日志…</div> : filtered.length === 0 ? <div className="empty-state"><div className="empty-icon"><ScrollText size={23} /></div><strong>{query ? '没有匹配的日志' : '还没有访问记录'}</strong><p>{query ? '尝试其他关键词。' : '当用户通过代理访问服务时，这里会显示鉴权结果。'}</p></div> : <div className="table-wrap"><table><thead><tr><th>时间</th><th>结果</th><th>服务</th><th>用户</th><th>请求</th><th>来源 IP</th><th>User-Agent</th></tr></thead><tbody>{filtered.map(log => { const meta = logStatus(log.status); return <tr key={log.id}><td>{new Date(log.created_at).toLocaleString('zh-CN')}</td><td><span className={`status-pill ${meta.cls}`}><span />{meta.label}</span></td><td>{log.service_name ?? log.service_id}</td><td>{log.username ?? '匿名'}</td><td><div className="request-cell"><code className="method-chip">{log.method ?? 'GET'}</code><code className="target-url">{log.path ?? '—'}</code></div></td><td className="mono-cell">{log.client_ip ?? '—'}</td><td className="ua-cell" title={log.user_agent ?? ''}>{log.user_agent ?? '—'}</td></tr> })}</tbody></table></div>}
      <div className="panel-footer"><span>共保留最近 5000 条记录</span><span>{auto ? '自动刷新已开启' : '自动刷新已关闭'}</span></div>
    </section>
    <div className="info-banner"><div className="info-symbol">i</div><div><strong>日志说明</strong><p>日志由 Caddy 转发鉴权请求时写入，反映的是访问决策结果；上游服务的具体响应状态不由 TunnelHub 记录。</p></div></div>
  </section>
}

function SettingsPage() {
  const [info, setInfo] = useState<NetworkInfo | null>(null)
  const [error, setError] = useState('')
  useEffect(() => {
    void api<NetworkInfo>('/api/services/network').then(setInfo).catch(e => setError(e instanceof Error ? e.message : '无法加载配置'))
  }, [])
  return <section className="page-content">
    <div className="page-heading"><div><div className="eyebrow">PLATFORM SETTINGS</div><h1>平台设置</h1><p>查看管理员会话与安全相关配置。</p></div></div>
    {error && <div className="alert error-alert"><span>{error}</span></div>}
    <section className="panel">
      <div className="panel-heading"><div><h2>安全与会话</h2><p>由环境变量控制，修改后需重启 TunnelHub</p></div></div>
      <div className="config-list">
        <ConfigRow label="管理域名" value={info?.ui_host ?? '任意域名均可访问'} />
        <ConfigRow label="服务域名" value={info?.service_domain ?? '未启用子域名路由'} />
        <ConfigRow label="Cookie Secure" value={info ? (info.cookie_secure ? '开启' : '关闭') : '—'} />
        <ConfigRow label="会话有效期" value="8 小时" />
        <ConfigRow label="版本" value={info?.version ?? '—'} />
      </div>
    </section>
    <div className="info-banner"><div className="info-symbol">i</div><div><strong>关于平台设置</strong><p>当前版本通过环境变量完成配置（见 README 与 .env.example）。可视化编辑将在后续版本开放。</p></div></div>
  </section>
}

const ROUTES = ['services', 'access', 'network', 'logs', 'settings'] as const
type Route = typeof ROUTES[number]

const ROUTE_TITLES: Record<Route, string> = {
  services: '服务管理',
  access: '用户与权限',
  network: '网络组件',
  logs: '访问日志',
  settings: '平台设置',
}

const NAV_ITEMS: { key: Route; label: string; icon: ReactNode; badge?: string }[] = [
  { key: 'services', label: '服务管理', icon: <LayoutDashboard size={18} /> },
  { key: 'access', label: '用户与权限', icon: <ShieldCheck size={18} /> },
  { key: 'network', label: '网络组件', icon: <Network size={18} />, badge: 'Caddy 内置' },
  { key: 'logs', label: '访问日志', icon: <ScrollText size={18} /> },
]

function useHashRoute(): Route {
  const parse = (): Route => {
    const raw = window.location.hash.replace(/^#\/?/, '').split('?')[0]
    return (ROUTES as readonly string[]).includes(raw) ? raw as Route : 'services'
  }
  const [route, setRoute] = useState<Route>(parse)
  useEffect(() => {
    const onChange = () => setRoute(parse())
    window.addEventListener('hashchange', onChange)
    return () => window.removeEventListener('hashchange', onChange)
  }, [])
  return route
}

function ServicesPage({ services, loading, error, refresh }: { services: Service[]; loading: boolean; error: string; refresh: () => Promise<void> }) {
  const [serviceDomain, setServiceDomain] = useState('')
  const [query, setQuery] = useState('')
  const [dialog, setDialog] = useState(false)
  const [editing, setEditing] = useState<Service | null>(null)
  const [draft, setDraft] = useState<Draft>(emptyDraft)
  const [saving, setSaving] = useState(false)
  const [testingTarget, setTestingTarget] = useState(false)
  const [targetTestResult, setTargetTestResult] = useState('')
  const [notice, setNotice] = useState('')
  const [openIds, setOpenIds] = useState<string[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [viewerMaximized, setViewerMaximized] = useState(false)

  useEffect(() => {
    void api<{ service_domain?: string | null }>('/api/health')
      .then((health) => setServiceDomain(health.service_domain || ''))
      .catch(() => setServiceDomain(''))
  }, [])

  useEffect(() => {
    if (!viewerMaximized) return
    const previous = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') setViewerMaximized(false) }
    window.addEventListener('keydown', onKey)
    return () => {
      document.body.style.overflow = previous
      window.removeEventListener('keydown', onKey)
    }
  }, [viewerMaximized])

  const serviceOrigin = (id: string) => {
    // Without an explicit service domain, use same-origin path routing so the
    // embedded view works on any host (for example a domain exposed through FRP).
    if (!serviceDomain) return `${window.location.origin}/proxy/${id}`
    const port = window.location.port ? `:${window.location.port}` : ''
    return `${window.location.protocol}//${id}.${serviceDomain}${port}`
  }

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return services
    return services.filter((item) => [item.name, item.target_url].some((value) => value.toLowerCase().includes(needle)))
  }, [services, query])

  function openCreate() {
    setEditing(null)
    setDraft(emptyDraft)
    setTargetTestResult('')
    setDialog(true)
  }
  function openEdit(service: Service) {
    setEditing(service)
    setDraft({ name: service.name, target_url: service.target_url, enabled: service.enabled })
    setTargetTestResult('')
    setDialog(true)
  }
  async function testTarget() {
    setTestingTarget(true)
    setTargetTestResult('')
    try {
      const result = await api<TargetTestResult>('/api/services/test-target', {
        method: 'POST',
        body: JSON.stringify({ target_url: draft.target_url }),
      })
      const contentType = result.content_type ? `，类型：${result.content_type}` : ''
      setTargetTestResult(`HTTP ${result.status}，TunnelHub 可访问目标服务${contentType}。最终地址：${result.final_url}`)
    } catch (e) {
      setTargetTestResult(e instanceof Error ? e.message : 'HTTP 连通性检测失败')
    } finally {
      setTestingTarget(false)
    }
  }
  async function save(event: FormEvent) {
    event.preventDefault()
    setSaving(true)
    try {
      if (editing) {
        await api<Service>(`/api/services/${editing.id}`, { method: 'PUT', body: JSON.stringify(draft) })
        setNotice('服务配置已更新')
      } else {
        await api<Service>('/api/services', { method: 'POST', body: JSON.stringify(draft) })
        setNotice('服务已添加')
      }
      setDialog(false)
      await refresh()
    } catch (e) {
      setNotice(e instanceof Error ? e.message : '保存失败')
    } finally {
      setSaving(false)
    }
  }
  async function toggle(service: Service) {
    try {
      await api<Service>(`/api/services/${service.id}`, { method: 'PUT', body: JSON.stringify({ enabled: !service.enabled }) })
      await refresh()
    } catch (e) {
      setNotice(e instanceof Error ? e.message : '更新状态失败')
    }
  }
  async function remove(service: Service) {
    if (!window.confirm(`确定删除服务“${service.name}”吗？`)) return
    try {
      await api<void>(`/api/services/${service.id}`, { method: 'DELETE' })
      setNotice('服务已删除')
      closeService(service.id)
      await refresh()
    } catch (e) {
      setNotice(e instanceof Error ? e.message : '删除失败')
    }
  }

  const openServices = useMemo(
    () => openIds.map(id => services.find(service => service.id === id)).filter((service): service is Service => Boolean(service)),
    [openIds, services],
  )
  const effectiveActiveId = openServices.some(service => service.id === activeId) ? activeId : (openServices[0]?.id ?? null)

  function openService(service: Service) {
    setOpenIds(ids => (ids.includes(service.id) ? ids : [...ids, service.id]))
    setActiveId(service.id)
  }
  function closeService(id: string) {
    const next = openIds.filter(value => value !== id)
    setOpenIds(next)
    if (activeId === id) setActiveId(next.length ? next[next.length - 1] : null)
    if (next.length === 0) setViewerMaximized(false)
  }
  function closeAllServices() {
    setOpenIds([])
    setActiveId(null)
    setViewerMaximized(false)
  }

  const enabledCount = services.filter((item) => item.enabled).length

  return <>
    <section className="page-content">
      {openServices.length > 0 && <section className={`panel embedded-viewer${viewerMaximized ? ' maximized' : ''}`}>
        <div className="panel-heading viewer-heading">
          <div className="viewer-tabs">{openServices.map(service => <div key={service.id} className={`viewer-tab${service.id === effectiveActiveId ? ' active' : ''}`}><button className="viewer-tab-label" title={serviceOrigin(service.id)} onClick={() => setActiveId(service.id)}>{service.name}</button><button className="viewer-tab-close" aria-label={`关闭 ${service.name}`} onClick={() => closeService(service.id)}><X size={13} /></button></div>)}</div>
          <div className="viewer-actions"><button className="secondary-button" onClick={() => setViewerMaximized(value => !value)}>{viewerMaximized ? <><Minimize2 size={15} />还原</> : <><Maximize2 size={15} />最大化</>}</button><button className="secondary-button" onClick={closeAllServices}>全部关闭</button></div>
        </div>
        <div className="viewer-frames">{openServices.map(service => <iframe key={service.id} title={service.name} src={`${serviceOrigin(service.id)}/`} className={service.id === effectiveActiveId ? undefined : 'hidden'} />)}</div>
      </section>}
      <div className="page-heading"><div><div className="eyebrow">NETWORK CONTROL PLANE</div><h1>服务管理</h1><p>登记可通过 HTTP 访问的目标服务，并通过同源路径内嵌打开；无需为服务单独配置域名。</p></div><button className="primary-button" onClick={openCreate}><Plus size={17} />添加服务</button></div>

      {error && <div className="alert error-alert"><span>{error}</span></div>}
      {notice && <div className="alert notice-alert"><span>{notice}</span><button onClick={() => setNotice('')} aria-label="关闭"><X size={16} /></button></div>}

      <div className="stats-grid">
        <div className="stat-card"><div className="stat-label">已配置服务</div><div className="stat-value">{services.length}<span>个</span></div><div className="stat-foot"><Globe2 size={14} />所有已登记的服务</div></div>
        <div className="stat-card"><div className="stat-label">已启用</div><div className="stat-value">{enabledCount}<span>个</span></div><div className="stat-foot"><span className="mini-dot green" />已登记并启用</div></div>
        <div className="stat-card"><div className="stat-label">代理引擎</div><div className="stat-value">Caddy</div><div className="stat-foot">自动同步服务路由</div></div>
      </div>

      <section className="panel">
        <div className="panel-heading"><div><h2>全部服务</h2><p>目标地址由 TunnelHub 通过 HTTP/HTTPS 访问，浏览器通过同源代理路径打开</p></div><button className="icon-button" onClick={() => void refresh()} title="刷新列表"><RefreshCw size={16} className={loading ? 'spin' : ''} /></button></div>
        <div className="table-toolbar"><div className="search-box"><Search size={16} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索服务名称或目标地址" /></div><span className="result-count">{filtered.length} 项</span></div>
        {loading ? <div className="empty-state">正在加载服务配置…</div> : filtered.length === 0 ? <div className="empty-state"><div className="empty-icon"><Globe2 size={23} /></div><strong>{query ? '没有匹配的服务' : '还没有配置服务'}</strong><p>{query ? '尝试其他关键词。' : '添加第一个服务，开始整理你的内网访问入口。'}</p>{!query && <button className="secondary-button" onClick={openCreate}><Plus size={16} />添加第一个服务</button>}</div> : <div className="table-wrap"><table><thead><tr><th>服务</th><th>内嵌访问路径</th><th>目标地址</th><th>状态</th><th className="actions-heading">操作</th></tr></thead><tbody>{filtered.map((service) => <tr key={service.id}><td><div className="service-name"><div className="service-icon"><Globe2 size={17} /></div><div><strong>{service.name}</strong><small>创建于 {new Date(service.created_at).toLocaleDateString('zh-CN')}</small></div></div></td><td><code className="target-url">{serviceOrigin(service.id).replace(/^https?:\/\//, '')}</code></td><td><code className="target-url">{service.target_url}</code></td><td><button className={`status-pill ${service.enabled ? 'enabled' : 'disabled'}`} onClick={() => void toggle(service)}><span />{service.enabled ? '已启用' : '已停用'}</button></td><td><div className="row-actions"><button className="icon-button" title="编辑服务" onClick={() => openEdit(service)}><Settings2 size={16} /></button><button className="icon-button danger-button" title="删除服务" onClick={() => void remove(service)}><Trash2 size={16} /></button><button className="icon-button" title="在页面内打开服务" onClick={() => openService(service)}><ArrowUpRight size={16} /></button></div></td></tr>)}</tbody></table></div>}
        <div className="panel-footer"><span>配置保存在本地 SQLite 数据库中</span><span>Caddy 路由自动同步</span></div>
      </section>
      <div className="info-banner"><div className="info-symbol">i</div><div><strong>当前版本说明</strong><p>TunnelHub 使用 Caddy 代理业务流量，无需为每个服务配置域名。FRP 与 DNS 仍由你自行配置；部分应用可能需要调整基础路径才能完整嵌入。</p></div></div>
    </section>

    {dialog && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setDialog(false) }}><section className="modal" role="dialog" aria-modal="true" aria-labelledby="dialog-title"><div className="modal-header"><div><h2 id="dialog-title">{editing ? '编辑服务' : '添加服务'}</h2><p>填写服务名称和容器可访问的目标地址</p></div><button className="icon-button" onClick={() => setDialog(false)} aria-label="关闭"><X size={18} /></button></div><form onSubmit={save}><label>服务名称<input autoFocus required maxLength={200} value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder="例如：Home Assistant" /></label><label>目标 HTTP 地址<input required type="text" inputMode="url" autoComplete="off" value={draft.target_url} onChange={(e) => { setDraft({ ...draft, target_url: e.target.value }); setTargetTestResult('') }} placeholder="http://host.docker.internal:8123" /><small>已默认填入宿主机前缀 http://host.docker.internal:，只需补上端口（例如 8123）；也可改成局域网地址，如 http://192.168.1.20:8080。</small><div className="target-test-row"><button type="button" className="secondary-button" onClick={() => void testTarget()} disabled={testingTarget || !draft.target_url.trim()}>{testingTarget ? '检测中…' : '测试 HTTP 连通性'}</button>{targetTestResult && <small role="status">{targetTestResult}</small>}</div></label><label className="checkbox-label"><input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />启用此服务配置</label><div className="modal-actions"><button type="button" className="secondary-button" onClick={() => setDialog(false)}>取消</button><button type="submit" className="primary-button" disabled={saving}>{saving ? '保存中…' : editing ? '保存修改' : '添加服务'}</button></div></form></section></div>}
  </>
}

function Dashboard() {
  const route = useHashRoute()
  const [services, setServices] = useState<Service[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  const load = useCallback(async () => {
    setLoading(true)
    try {
      setServices(await api<Service[]>('/api/services'))
      setError('')
    } catch (e) {
      setError(e instanceof Error ? e.message : '无法加载服务列表')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { void load() }, [load])

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><div className="brand-mark"><Waypoints size={21} /></div><span>Tunnel<span className="brand-light">Hub</span></span></div>
        <div className="workspace-label">工作空间</div>
        {NAV_ITEMS.map(item => <a key={item.key} href={`#/${item.key}`} className={route === item.key ? 'nav-item active' : 'nav-item'}>{item.icon}{item.label}{item.badge && <span className="soon">{item.badge}</span>}</a>)}
        <div className="sidebar-bottom">
          <div className="connection-card"><span className="status-dot" /><div><strong>管理 API</strong><small>通过本地 API 连接</small></div></div>
          <a href="#/settings" className={route === 'settings' ? 'nav-item active' : 'nav-item'}><Settings2 size={18} />平台设置</a>
          <div className="version">TUNNELHUB <span>v0.1.0</span></div>
        </div>
      </aside>

      <main className="main">
        <header className="topbar"><div className="breadcrumbs">工作空间 <span>/</span> <strong>{ROUTE_TITLES[route]}</strong></div><div className="topbar-actions"><button className="icon-button" title="关于 TunnelHub" onClick={() => window.alert('TunnelHub：统一管理内网服务，并在页面内嵌访问目标服务。')}><CircleHelp size={18} /></button><button className="logout-button" onClick={async () => { await fetch('/api/auth/logout', { method: 'POST' }); window.location.reload() }}><LogOut size={15} />退出登录</button></div></header>
        {route === 'services' && <ServicesPage services={services} loading={loading} error={error} refresh={load} />}
        {route === 'access' && <AccessControlPanel services={services} />}
        {route === 'network' && <NetworkPage />}
        {route === 'logs' && <LogsPage />}
        {route === 'settings' && <SettingsPage />}
      </main>
    </div>
  )
}


type AuthStatus = { setup_required: boolean; authenticated: boolean; totp_pending: boolean }
type TotpSetup = { secret: string; otpauth_uri: string }
type RecoveryResponse = { recovery_codes: string[] }

async function authRequest<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method: body === undefined ? 'GET' : 'POST',
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: 'same-origin',
  })
  if (!response.ok) {
    const payload = await response.json().catch(() => null)
    throw new Error(payload?.error ?? `请求失败（${response.status}）`)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

export default function App() {
  const [setupInfo, setSetupInfo] = useState<TotpSetup | null>(null)
  const [view, setView] = useState<'loading' | 'setup' | 'totp' | 'login' | 'recovery' | 'authenticated'>('loading')
  const [recoveryCodes, setRecoveryCodes] = useState<string[]>([])
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [authError, setAuthError] = useState('')
  const [busy, setBusy] = useState(false)

  const refreshAuth = useCallback(async () => {
    try {
      const current = await authRequest<AuthStatus>('/api/auth/status')
      if (current.authenticated) {
        setView('authenticated')
      } else if (current.setup_required) {
        setView('setup')
      } else if (current.totp_pending) {
        const info = await authRequest<TotpSetup>('/api/auth/totp/pending')
        setSetupInfo(info)
        setView('totp')
      } else {
        setView('login')
      }
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : '无法检查登录状态')
      setView('login')
    }
  }, [])

  useEffect(() => { void refreshAuth() }, [refreshAuth])

  async function submitSetup(event: React.FormEvent) {
    event.preventDefault()
    setBusy(true); setAuthError('')
    try {
      const info = await authRequest<TotpSetup>('/api/auth/setup', { username, password })
      setSetupInfo(info)
      setView('totp')
      setCode('')
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : '初始化管理员失败')
    } finally { setBusy(false) }
  }

  async function submitTotp(event: React.FormEvent) {
    event.preventDefault()
    setBusy(true); setAuthError('')
    try {
      const result = await authRequest<RecoveryResponse>('/api/auth/totp/confirm', { code })
      setRecoveryCodes(result.recovery_codes)
      setView('recovery')
      setCode('')
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : 'TOTP 验证失败')
    } finally { setBusy(false) }
  }

  async function submitLogin(event: React.FormEvent) {
    event.preventDefault()
    setBusy(true); setAuthError('')
    try {
      await authRequest('/api/auth/login', { username, password, code })
      setView('authenticated')
    } catch (error) {
      setAuthError(error instanceof Error ? error.message : '登录失败')
    } finally { setBusy(false) }
  }

  if (view === 'loading') return <div className="auth-loading">正在检查登录状态…</div>
  if (view === 'authenticated') return <Dashboard />

  return (
    <main className="auth-shell">
      <section className="auth-card">
        <div className="auth-brand"><div className="brand-mark"><Waypoints size={21} /></div><span>Tunnel<span className="brand-light">Hub</span></span></div>
        {authError && <div className="alert error-alert auth-error">{authError}</div>}
        {view === 'setup' && <>
          <div className="auth-eyebrow">FIRST-TIME SETUP</div>
          <h1>初始化管理员</h1>
          <p className="auth-description">创建唯一管理员账户。密码至少 12 个字符，随后必须绑定 TOTP 才能使用管理界面。</p>
          <form className="auth-form" onSubmit={submitSetup}>
            <label>管理员用户名<input autoComplete="username" required minLength={3} maxLength={64} value={username} onChange={e => setUsername(e.target.value)} /></label>
            <label>管理员密码<input type="password" autoComplete="new-password" required minLength={12} value={password} onChange={e => setPassword(e.target.value)} /></label>
            <button className="primary-button auth-submit" disabled={busy}>{busy ? '正在初始化…' : '创建管理员并设置 TOTP'}</button>
          </form>
        </>}
        {view === 'totp' && setupInfo && <>
          <div className="auth-eyebrow">TWO-FACTOR AUTHENTICATION</div>
          <h1>绑定验证器</h1>
          <p className="auth-description">在 Aegis、Google Authenticator 或其他兼容应用中添加账户。可扫描 URI（若你的验证器支持）或手动输入密钥。</p>
          <div className="totp-qr"><QRCodeSVG value={setupInfo.otpauth_uri} size={176} level="M" /></div><div className="totp-secret"><span>密钥</span><code>{setupInfo.secret}</code><button type="button" className="secondary-button" onClick={() => void navigator.clipboard?.writeText(setupInfo.secret)}>复制密钥</button></div>
          <details className="otpauth-details"><summary>显示 otpauth URI</summary><code>{setupInfo.otpauth_uri}</code></details>
          <form className="auth-form" onSubmit={submitTotp}>
            <label>6 位动态验证码<input inputMode="numeric" autoComplete="one-time-code" pattern="[0-9]{6}" maxLength={6} required value={code} onChange={e => setCode(e.target.value.replace(/\D/g, '').slice(0, 6))} /></label>
            <button className="primary-button auth-submit" disabled={busy}>{busy ? '正在验证…' : '确认绑定并启用 TOTP'}</button>
          </form>
        </>}
        {view === 'login' && <>
          <div className="auth-eyebrow">SECURE ADMIN ACCESS</div>
          <h1>管理员登录</h1>
          <p className="auth-description">使用管理员密码和验证器中的 6 位动态验证码登录。</p>
          <form className="auth-form" onSubmit={submitLogin}>
            <label>用户名<input autoComplete="username" required value={username} onChange={e => setUsername(e.target.value)} /></label>
            <label>密码<input type="password" autoComplete="current-password" required value={password} onChange={e => setPassword(e.target.value)} /></label>
            <label>动态验证码或恢复码<input autoComplete="one-time-code" required value={code} onChange={e => setCode(e.target.value.trim().slice(0, 32))} placeholder="6 位验证码 / 恢复码" /></label>
            <button className="primary-button auth-submit" disabled={busy}>{busy ? '正在登录…' : '登录'}</button>
          </form>
        </>}
        {view === 'recovery' && <>
          <div className="auth-eyebrow">SAVE THESE CODES</div>
          <h1>保存恢复码</h1>
          <p className="auth-description">以下恢复码每个只能使用一次。请立即保存到密码管理器或其他安全位置；离开此页后不会再次显示。</p>
          <div className="recovery-grid">{recoveryCodes.map(item => <code key={item}>{item}</code>)}</div>
          <button className="primary-button auth-submit" onClick={() => { setRecoveryCodes([]); setView('authenticated') }}>我已安全保存，进入管理界面</button>
        </>}
        <div className="auth-footer">会话有效期 8 小时 · TOTP 动态验证码每 30 秒更新</div>
      </section>
    </main>
  )
}