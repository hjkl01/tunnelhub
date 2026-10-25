import { useCallback, useEffect, useMemo, useState } from 'react'
import { QRCodeSVG } from 'qrcode.react'
import { Activity, ArrowUpRight, CircleHelp, Globe2, LayoutDashboard, LogOut, Plus, RefreshCw, Search, Settings2, ShieldCheck, Trash2, Waypoints, X } from 'lucide-react'

type Service = {
  id: string
  name: string
  hostname: string
  target_url: string
  enabled: boolean
  created_at: string
  updated_at: string
  access_mode: string
}
type Draft = { name: string; hostname?: string; target_url: string; enabled: boolean }
const emptyDraft: Draft = { name: '', target_url: 'http://host.docker.internal:3000', enabled: true }

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
  return <section id="access-control" className="page-content access-page">
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

function ServiceDashboard() {
  const [services, setServices] = useState<Service[]>([])
  const [section, setSection] = useState('services')
  const [selectedService, setSelectedService] = useState<Service | null>(null)
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [dialog, setDialog] = useState(false)
  const [editing, setEditing] = useState<Service | null>(null)
  const [draft, setDraft] = useState<Draft>(emptyDraft)
  const [saving, setSaving] = useState(false)
  const [notice, setNotice] = useState('')

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

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return services
    return services.filter((item) => [item.name, item.target_url].some((value) => value.toLowerCase().includes(needle)))
  }, [services, query])

  function openCreate() {
    setEditing(null)
    setDraft(emptyDraft)
    setDialog(true)
  }
  function openEdit(service: Service) {
    setEditing(service)
    setDraft({ name: service.name, target_url: service.target_url, enabled: service.enabled })
    setDialog(true)
  }
  async function save(event: React.FormEvent) {
    event.preventDefault()
    setSaving(true)
    setError('')
    try {
      if (editing) {
        await api<Service>(`/api/services/${editing.id}`, { method: 'PUT', body: JSON.stringify(draft) })
        setNotice('服务配置已更新')
      } else {
        await api<Service>('/api/services', { method: 'POST', body: JSON.stringify(draft) })
        setNotice('服务已添加')
      }
      setDialog(false)
      await load()
    } catch (e) {
      setError(e instanceof Error ? e.message : '保存失败')
    } finally {
      setSaving(false)
    }
  }
  async function toggle(service: Service) {
    try {
      await api<Service>(`/api/services/${service.id}`, {
        method: 'PUT',
        body: JSON.stringify({ enabled: !service.enabled }),
      })
      await load()
    } catch (e) {
      setError(e instanceof Error ? e.message : '更新状态失败')
    }
  }
  async function remove(service: Service) {
    if (!window.confirm(`确定删除服务“${service.name}”吗？`)) return
    try {
      await api<void>(`/api/services/${service.id}`, { method: 'DELETE' })
      setNotice('服务已删除')
      await load()
    } catch (e) {
      setError(e instanceof Error ? e.message : '删除失败')
    }
  }

  const enabledCount = services.filter((item) => item.enabled).length

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><div className="brand-mark"><Waypoints size={21} /></div><span>Tunnel<span className="brand-light">Hub</span></span></div>
        <div className="workspace-label">工作空间</div>
        <button className={section === "services" ? "nav-item active" : "nav-item"} onClick={() => setSection("services")}><LayoutDashboard size={18} />服务管理</button>
        <a className="nav-item" href="#access-control"><ShieldCheck size={18} />用户与权限</a>
        <button className="nav-item" onClick={() => setNotice('Caddy 负责服务转发，FRP 仍由你自行配置。')}><Activity size={18} />网络组件 <span className="soon">Caddy 内置</span></button>
        <button className="nav-item" onClick={() => setNotice('访问日志将在后续版本开放')}><ShieldCheck size={18} />访问日志 <span className="soon">即将推出</span></button>
        <div className="sidebar-bottom">
          <div className="connection-card"><span className="status-dot" /><div><strong>管理 API</strong><small>通过本地 API 连接</small></div></div>
          <button className="nav-item" onClick={() => setNotice('平台设置将在后续版本开放')}><Settings2 size={18} />平台设置</button>
          <div className="version">TUNNELHUB <span>v0.1.0</span></div>
        </div>
      </aside>

      <main className="main">
        <header className="topbar"><div className="breadcrumbs">工作空间 <span>/</span> <strong>服务管理</strong></div><div className="topbar-actions"><button className="icon-button" title="关于 TunnelHub" onClick={() => setNotice('TunnelHub：统一管理内网服务，并在页面内嵌访问目标服务。')}><CircleHelp size={18} /></button><button className="logout-button" onClick={async () => { await fetch('/api/auth/logout', { method: 'POST' }); window.location.reload() }}><LogOut size={15} />退出登录</button></div></header>
        <section className="page-content">
          {selectedService && <section className="panel embedded-viewer"><div className="panel-heading"><div><h2>{selectedService.name}</h2><p>/proxy/{selectedService.id}/</p></div><button className="secondary-button" onClick={() => setSelectedService(null)}>关闭视图</button></div><iframe title={selectedService.name} src={`/proxy/${selectedService.id}/`} /></section>}
          <div className="page-heading"><div><div className="eyebrow">NETWORK CONTROL PLANE</div><h1>服务管理</h1><p>统一登记服务地址与访问域名；公网映射由你自行配置 FRP。</p></div><button className="primary-button" onClick={openCreate}><Plus size={17} />添加服务</button></div>

          {error && <div className="alert error-alert"><span>{error}</span><button onClick={() => setError('')} aria-label="关闭"><X size={16} /></button></div>}
          {notice && <div className="alert notice-alert"><span>{notice}</span><button onClick={() => setNotice('')} aria-label="关闭"><X size={16} /></button></div>}

          <div className="stats-grid">
            <div className="stat-card"><div className="stat-label">已配置服务</div><div className="stat-value">{services.length}<span>个</span></div><div className="stat-foot"><Globe2 size={14} />所有已登记的服务</div></div>
            <div className="stat-card"><div className="stat-label">已启用</div><div className="stat-value">{enabledCount}<span>个</span></div><div className="stat-foot"><span className="mini-dot green" />已登记并启用</div></div>
            <div className="stat-card"><div className="stat-label">代理引擎</div><div className="stat-value">Caddy</div><div className="stat-foot">自动同步服务路由</div></div>
          </div>

          <section className="panel">
            <div className="panel-heading"><div><h2>全部服务</h2><p>管理内嵌访问入口与目标地址</p></div><button className="icon-button" onClick={() => void load()} title="刷新列表"><RefreshCw size={16} className={loading ? 'spin' : ''} /></button></div>
            <div className="table-toolbar"><div className="search-box"><Search size={16} /><input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索服务名称或目标地址" /></div><span className="result-count">{filtered.length} 项</span></div>
            {loading ? <div className="empty-state">正在加载服务配置…</div> : filtered.length === 0 ? <div className="empty-state"><div className="empty-icon"><Globe2 size={23} /></div><strong>{query ? '没有匹配的服务' : '还没有配置服务'}</strong><p>{query ? '尝试其他关键词。' : '添加第一个服务，开始整理你的内网访问入口。'}</p>{!query && <button className="secondary-button" onClick={openCreate}><Plus size={16} />添加第一个服务</button>}</div> : <div className="table-wrap"><table><thead><tr><th>服务</th><th>内嵌访问路径</th><th>目标地址</th><th>状态</th><th className="actions-heading">操作</th></tr></thead><tbody>{filtered.map((service) => <tr key={service.id}><td><div className="service-name"><div className="service-icon"><Globe2 size={17} /></div><div><strong>{service.name}</strong><small>创建于 {new Date(service.created_at).toLocaleDateString('zh-CN')}</small></div></div></td><td><code className="target-url">/proxy/{service.id}/</code></td><td><code className="target-url">{service.target_url}</code></td><td><button className={`status-pill ${service.enabled ? 'enabled' : 'disabled'}`} onClick={() => void toggle(service)}><span />{service.enabled ? '已启用' : '已停用'}</button></td><td><div className="row-actions"><button className="icon-button" title="编辑服务" onClick={() => openEdit(service)}><Settings2 size={16} /></button><button className="icon-button danger-button" title="删除服务" onClick={() => void remove(service)}><Trash2 size={16} /></button><button className="icon-button" title="在页面内打开服务" onClick={() => setSelectedService(service)}><ArrowUpRight size={16} /></button></div></td></tr>)}</tbody></table></div>}
            <div className="panel-footer"><span>配置保存在本地 SQLite 数据库中</span><span>Caddy 路由自动同步</span></div>
          </section>
          <div className="info-banner"><div className="info-symbol">i</div><div><strong>当前版本说明</strong><p>TunnelHub 使用 Caddy 代理业务流量，无需为每个服务配置域名。FRP 与 DNS 仍由你自行配置；部分应用可能需要调整基础路径才能完整嵌入。</p></div></div>
        </section>
        <AccessControlPanel services={services} />
      </main>

      {dialog && <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) setDialog(false) }}><section className="modal" role="dialog" aria-modal="true" aria-labelledby="dialog-title"><div className="modal-header"><div><h2 id="dialog-title">{editing ? '编辑服务' : '添加服务'}</h2><p>填写服务名称和容器可访问的目标地址</p></div><button className="icon-button" onClick={() => setDialog(false)} aria-label="关闭"><X size={18} /></button></div><form onSubmit={save}><label>服务名称<input autoFocus required maxLength={200} value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder="例如：Home Assistant" /></label><label>旧版访问域名（可选）<input value={draft.hostname ?? ''} onChange={(e) => setDraft({ ...draft, hostname: e.target.value })} placeholder="留空即可，例如 ha.example.com" /><small>仅用于兼容旧版按 Host 访问；新服务无需填写。</small></label><label>目标 URL<input required type="url" value={draft.target_url} onChange={(e) => setDraft({ ...draft, target_url: e.target.value })} placeholder="http://host.docker.internal:8123" /><small>填写目标服务地址，必须包含 http:// 或 https://。容器访问宿主机时使用 host.docker.internal。</small></label><label className="checkbox-label"><input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />启用此服务配置</label><div className="modal-actions"><button type="button" className="secondary-button" onClick={() => setDialog(false)}>取消</button><button type="submit" className="primary-button" disabled={saving}>{saving ? '保存中…' : editing ? '保存修改' : '添加服务'}</button></div></form></section></div>}
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
  if (view === 'authenticated') return <ServiceDashboard />

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
