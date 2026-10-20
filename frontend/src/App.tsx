import { useEffect, useRef, useState } from 'react'
import { readCuts, removedDuration, type Cut } from './editing'

type Project = { id: string; name: string }
type MediaEntry = {
  file: { id: string; original_name: string; size_bytes: number; status: string }
  metadata: { duration_ms: number; width?: number; height?: number; video_codec?: string; audio_codec?: string; sample_rate?: number; channels?: number } | null
}
type Analysis = { id: string; media_file_id: string; kind: string; start_ms: number; end_ms: number; confidence: number }
type Job = { id: string; type: string; status: string; progress: number; current_step?: string; error?: string }
type Render = { id: string; status: string; rendered_at: string | null }
type History = { past: Cut[][]; present: Cut[]; future: Cut[][] }
const API = (import.meta.env.VITE_API_URL ?? '').replace(/\/$/, '')
const emptyHistory = (): History => ({ past: [], present: [], future: [] })
const json = (value: unknown) => ({ headers: { 'content-type': 'application/json' }, body: JSON.stringify(value) })
const label = (kind: string) => ({ silence: 'Silêncio', black: 'Tela preta', freeze: 'Imagem congelada', low_volume: 'Volume baixo' }[kind] ?? kind)
const isCut = (item: Analysis) => ['silence', 'black', 'freeze'].includes(item.kind) && item.end_ms > item.start_ms
const clock = (ms = 0) => {
  const seconds = Math.floor(Math.max(0, ms) / 1000)
  return [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60].map(n => String(n).padStart(2, '0')).join(':')
}
async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const response = await fetch(`${API}${path}`, options)
  const data = await response.json().catch(() => ({}))
  if (!response.ok) throw new Error(data.error ?? `Falha HTTP ${response.status}`)
  return data as T
}

export default function App() {
  const fileInput = useRef<HTMLInputElement>(null)
  const projectDialog = useRef<HTMLDialogElement>(null)
  const player = useRef<HTMLVideoElement>(null)
  const pendingSeek = useRef(0)
  const activeEvents = useRef<EventSource | null>(null)
  const [projects, setProjects] = useState<Project[]>([])
  const [project, setProject] = useState<Project | null>(null)
  const [media, setMedia] = useState<MediaEntry[]>([])
  const [selectedId, setSelectedId] = useState('')
  const [analysis, setAnalysis] = useState<Analysis[]>([])
  const [renders, setRenders] = useState<Render[]>([])
  const [history, setHistory] = useState<History>(emptyHistory)
  const [job, setJob] = useState<Job | null>(null)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('Abra um projeto ou crie um novo para começar.')
  const [error, setError] = useState('')
  const [previewError, setPreviewError] = useState('')
  const [search, setSearch] = useState('')
  const [filter, setFilter] = useState('all')
  const [dragging, setDragging] = useState(false)
  const [playing, setPlaying] = useState(false)
  const [time, setTime] = useState(0)
  const [zoom, setZoom] = useState(1)
  const [normalize, setNormalize] = useState(true)
  const [cutStart, setCutStart] = useState(0)
  const [cutEnd, setCutEnd] = useState(0)
  const [projectForm, setProjectForm] = useState<'new' | 'rename' | null>(null)
  const [name, setName] = useState('')

  const selected = media.find(item => item.file.id === selectedId)
  const duration = selected?.metadata?.duration_ms ?? 0
  const total = media.reduce((sum, item) => sum + (item.metadata?.duration_ms ?? 0), 0)
  const cuts = history.present
  const removed = media.reduce((sum, item) => sum + removedDuration(cuts, item.file.id, item.metadata?.duration_ms ?? 0), 0)
  const offset = media.slice(0, Math.max(0, media.findIndex(item => item.file.id === selectedId))).reduce((sum, item) => sum + (item.metadata?.duration_ms ?? 0), 0)
  const filtered = media.filter(item => item.file.original_name.toLocaleLowerCase().includes(search.toLocaleLowerCase()) &&
    (filter === 'all' || (filter === 'video' ? item.metadata?.video_codec : item.metadata?.audio_codec && !item.metadata?.video_codec)))
  const renderProblem = media.some(item => !item.metadata?.video_codec || !item.metadata?.audio_codec || !item.metadata.duration_ms)
    ? 'Exportação disponível para arquivos com vídeo e áudio. Mídias sem uma dessas faixas podem ser reproduzidas e analisadas.'
    : media.length > 1 && cuts.length > 0 ? 'Nesta versão, cortes na exportação exigem um projeto com um arquivo. Vários arquivos podem ser concatenados sem cortes.'
    : total > 0 && total === removed ? 'Os cortes removem todo o conteúdo.' : ''

  function fail(cause: unknown) { setError(cause instanceof Error ? cause.message : 'Não foi possível concluir a operação.') }
  useEffect(() => {
    let mounted = true
    request<Project[]>('/api/projects').then(items => { if (mounted) setProjects(items) }).catch(cause => { if (mounted) fail(cause) })
    return () => { mounted = false; activeEvents.current?.close() }
  }, [])
  useEffect(() => { document.title = project ? `${project.name} — VideoLab` : 'VideoLab — Editor de vídeo' }, [project])

  useEffect(() => { if (projectForm) projectDialog.current?.showModal() }, [projectForm])

  async function refresh(id: string) {
    const [files, suggestions, outputs] = await Promise.all([
      request<MediaEntry[]>(`/api/projects/${id}/media`), request<Analysis[]>(`/api/projects/${id}/timeline`), request<Render[]>(`/api/projects/${id}/renders`),
    ])
    setMedia(files); setAnalysis(suggestions); setRenders(outputs)
    setSelectedId(current => files.some(item => item.file.id === current) ? current : files[0]?.file.id ?? '')
    return files
  }
  async function openProject(next: Project) {
    if (busy) return
    setBusy(true); setError('')
    try {
      const files = await refresh(next.id)
      setProject(next); setSelectedId(files[0]?.file.id ?? ''); setTime(0); pendingSeek.current = 0
      setPlaying(false); setPreviewError(''); setJob(null); setCutStart(0); setCutEnd(0); setSearch(''); setFilter('all')
      let saved: Cut[] = []
      try { saved = readCuts(localStorage.getItem(`videolab:cuts:${next.id}`)) } catch { /* Browser storage may be disabled. */ }
      saved = saved.filter(cut => files.some(item => item.file.id === cut.media_id && cut.end_ms <= (item.metadata?.duration_ms ?? 0)))
      setHistory({ ...emptyHistory(), present: saved }); setNotice('Projeto aberto. Selecione um clipe para revisar.')
    } catch (cause) { fail(cause) } finally { setBusy(false) }
  }
  async function saveProject() {
    if (!name.trim() || busy) return
    setBusy(true); setError('')
    try {
      if (projectForm === 'rename' && project) {
        await request(`/api/projects/${project.id}`, { method: 'PATCH', ...json({ name }) })
        setProject({ ...project, name: name.trim() })
      } else {
        const created = await request<Project>('/api/projects', { method: 'POST', ...json({ name }) })
        setProject(created); setMedia([]); setAnalysis([]); setRenders([]); setSelectedId(''); setHistory(emptyHistory()); setJob(null)
        setTime(0); pendingSeek.current = 0; setPlaying(false); setPreviewError(''); setCutStart(0); setCutEnd(0); setSearch(''); setFilter('all')
      }
      setProjects(await request<Project[]>('/api/projects')); setProjectForm(null); setNotice('Projeto salvo.')
    } catch (cause) { fail(cause) } finally { setBusy(false) }
  }
  function updateHistory(next: History) {
    setHistory(next)
    if (project) {
      try { localStorage.setItem(`videolab:cuts:${project.id}`, JSON.stringify(next.present)); setNotice('Revisão salva neste navegador.') }
      catch { setNotice('Revisão mantida nesta sessão; o navegador não permitiu salvá-la.') }
    }
  }
  function commit(next: Cut[]) { updateHistory({ past: [...history.past.slice(-49), cuts], present: next, future: [] }) }
  function undo() { if (history.past.length) updateHistory({ past: history.past.slice(0, -1), present: history.past.at(-1)!, future: [cuts, ...history.future] }) }
  function redo() { if (history.future.length) updateHistory({ past: [...history.past, cuts], present: history.future[0], future: history.future.slice(1) }) }
  function choose(id: string, position = 0) {
    setPreviewError(''); setCutStart(0); setCutEnd(0)
    if (id === selectedId && player.current) { player.current.currentTime = position / 1000; setTime(position) }
    else { pendingSeek.current = position; setSelectedId(id); setTime(position); setPlaying(false) }
  }
  function seek(position: number) { if (player.current) { player.current.currentTime = position / 1000; setTime(position) } }
  async function togglePlayback() {
    const element = player.current
    if (!element) return
    if (!element.paused) element.pause()
    else try { await element.play() } catch { setPreviewError('Este formato não pode ser reproduzido pelo navegador. Abra o original em um player compatível.') }
  }
  function addCut() {
    const start = Math.round(cutStart * 1000), end = Math.round(cutEnd * 1000)
    if (!selected || !Number.isFinite(start) || !Number.isFinite(end) || start < 0 || end <= start || end > duration) {
      setError('Informe início e fim dentro da duração do clipe, com fim maior que início.'); return
    }
    setError(''); commit([...cuts, { id: crypto.randomUUID(), media_id: selectedId, start_ms: start, end_ms: end }])
  }
  function toggleSuggestion(item: Analysis) {
    if (!isCut(item)) return
    commit(cuts.some(cut => cut.id === item.id) ? cuts.filter(cut => cut.id !== item.id) : [...cuts, { id: item.id, media_id: item.media_file_id, start_ms: item.start_ms, end_ms: item.end_ms }])
  }
  async function moveMedia(direction: number) {
    if (!project || busy) return
    const index = media.findIndex(item => item.file.id === selectedId), destination = index + direction
    if (index < 0 || destination < 0 || destination >= media.length) return
    const next = [...media]; [next[index], next[destination]] = [next[destination], next[index]]
    setBusy(true); setError('')
    try { await request(`/api/projects/${project.id}/media/order`, { method: 'PATCH', ...json({ media_ids: next.map(item => item.file.id) }) }); setMedia(next); setNotice('Ordem dos arquivos salva no projeto.') }
    catch (cause) { fail(cause) } finally { setBusy(false) }
  }
  async function watchJob(id: string) {
    await new Promise<void>((resolve, reject) => {
      const source = new EventSource(`${API}/api/jobs/${id}/events`)
      activeEvents.current = source
      const finish = (error?: Error) => { source.close(); activeEvents.current = null; error ? reject(error) : resolve() }
      source.addEventListener('job', event => {
        try {
          const next = JSON.parse((event as MessageEvent).data) as Job; setJob(next)
          if (next.status === 'COMPLETED') finish()
          else if (['FAILED', 'CANCELED'].includes(next.status)) finish(new Error(next.error ?? 'Processamento interrompido.'))
        } catch (cause) { finish(cause instanceof Error ? cause : new Error('Evento inválido')) }
      })
      source.onerror = () => {
        if (source.readyState === EventSource.CLOSED) finish(new Error('Conexão com o processamento perdida. Reabra o projeto para atualizar os resultados.'))
      }
    })
  }
  async function upload(files: FileList | null) {
    if (!project || !files?.length || busy) return
    setBusy(true); setError(''); setNotice('Importando mídias…')
    try {
      for (const file of Array.from(files)) {
        const form = new FormData(); form.append('file', file)
        const results = await request<{ job_id: string }[]>(`/api/projects/${project.id}/media`, { method: 'POST', body: form })
        for (const result of results) await watchJob(result.job_id)
      }
      setNotice('Importação concluída.')
    } catch (cause) { fail(cause) }
    finally { try { await refresh(project.id) } catch (cause) { fail(cause) }; setBusy(false) }
  }
  async function process(kind: 'analyze' | 'render') {
    if (!project || busy || !media.length) return
    setBusy(true); setError(''); setNotice(kind === 'analyze' ? 'Analisando mídias…' : 'Exportando vídeo…')
    try {
      const result = await request<{ job_id: string }>(`/api/projects/${project.id}/${kind}`, { method: 'POST', ...(kind === 'render' ? json({ manual_cuts: cuts, normalize_audio: normalize }) : {}) })
      await watchJob(result.job_id); setNotice('Processamento concluído.')
    } catch (cause) { fail(cause) }
    finally { try { await refresh(project.id) } catch (cause) { fail(cause) }; setBusy(false) }
  }
  const contentUrl = project && selected ? `${API}/api/projects/${project.id}/media/${selectedId}/content` : ''
  return <div className="app-shell">
    <header className="topbar">
      <a className="brand" href="/" aria-label="VideoLab — início"><span className="brand-mark">▤</span>VIDEOLAB</a>
      <div className="project-picker"><select aria-label="Projeto" value={project?.id ?? ''} disabled={busy} onChange={event => { const next = projects.find(item => item.id === event.target.value); if (next) void openProject(next) }}><option value="" disabled>Escolha um projeto</option>{projects.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select><button disabled={busy} onClick={() => { setProjectForm('new'); setName('') }}>+ Novo projeto</button><button disabled={!project || busy} onClick={() => { setProjectForm('rename'); setName(project?.name ?? '') }}>Renomear</button></div>
      <div className="top-actions"><button className="outline-button secondary-action" disabled={!media.length || busy} onClick={() => void process('analyze')}>✦ Analisar</button><button className="outline-button" disabled={!media.length || busy || !!renderProblem} title={renderProblem || 'Exportar na ordem da sequência, aplicando os cortes'} onClick={() => void process('render')}>↥ Exportar vídeo</button></div>
    </header>
    {projectForm && <dialog ref={projectDialog} aria-labelledby="project-form-title" className="project-dialog" onCancel={event => { if (busy) event.preventDefault(); else setProjectForm(null) }}><form onSubmit={event => { event.preventDefault(); void saveProject() }}><h2 id="project-form-title">{projectForm === 'new' ? 'Novo projeto' : 'Renomear projeto'}</h2><label>Nome do projeto<input autoFocus required maxLength={100} value={name} onChange={event => setName(event.target.value)} placeholder="Ex.: Entrevista, tutorial ou apresentação" /></label>{error && <p role="alert">{error}</p>}<div><button type="button" disabled={busy} onClick={() => setProjectForm(null)}>Cancelar</button><button className="outline-button" disabled={busy || !name.trim()}>Salvar projeto</button></div></form></dialog>}
    {error && !projectForm && <div className="error-banner" role="alert">{error}<button aria-label="Fechar mensagem" onClick={() => setError('')}>×</button></div>}
    <div className="app-body">
      <aside className="left-sidebar">
        <section className="panel project-panel"><div className="panel-title"><span>Mídias do projeto</span><span>{media.length}</span></div>
          <input ref={fileInput} type="file" multiple accept="video/*,audio/*,.mkv,.mov" hidden onChange={event => { void upload(event.target.files); event.target.value = '' }} />
          <label className="project-search"><span aria-hidden="true">⌕</span><input aria-label="Buscar mídia" placeholder="Buscar mídia…" value={search} onChange={event => setSearch(event.target.value)} /></label>
          <div className="bin-toolbar" aria-label="Tipo de mídia">{[['all', 'Tudo'], ['video', 'Vídeo'], ['audio', 'Áudio']].map(([value, text]) => <button key={value} aria-pressed={filter === value} className={filter === value ? 'active-filter' : ''} onClick={() => setFilter(value)}>{text}</button>)}</div>
          <div className="media-list">{filtered.map((item, index) => <button className={`media-item ${selectedId === item.file.id ? 'selected' : ''}`} key={item.file.id} aria-pressed={selectedId === item.file.id} onClick={() => choose(item.file.id)}><span className={`media-thumb ${['blue', 'violet', 'orange'][index % 3]}`} aria-hidden="true">{item.metadata?.video_codec ? '▤' : '♬'}<span className="thumb-duration">{clock(item.metadata?.duration_ms)}</span></span><span className="media-copy"><strong>{item.file.original_name}</strong><small>{item.metadata?.width ? `${item.metadata.width} × ${item.metadata.height}` : item.metadata?.audio_codec ?? item.file.status}</small></span></button>)}{!filtered.length && <p className="empty-media">{media.length ? 'Nenhuma mídia corresponde ao filtro.' : 'Seus vídeos e áudios aparecerão aqui.'}</p>}</div>
          <button className={`drop-zone ${dragging ? 'drag-active' : ''}`} disabled={!project || busy} onClick={() => fileInput.current?.click()} onDragOver={event => { event.preventDefault(); setDragging(true) }} onDragLeave={() => setDragging(false)} onDrop={event => { event.preventDefault(); setDragging(false); void upload(event.dataTransfer.files) }}><span>↥ {busy ? 'Processando…' : 'Importar mídias'}</span><small>{project ? 'Escolha ou arraste vídeos e áudios' : 'Crie ou abra um projeto para importar'}</small></button>
          <div className="order-actions"><button disabled={busy || !selected || media[0]?.file.id === selectedId} onClick={() => void moveMedia(-1)}>↑ Mover antes</button><button disabled={busy || !selected || media.at(-1)?.file.id === selectedId} onClick={() => void moveMedia(1)}>↓ Mover depois</button></div>
        </section>
        <section className="panel suggestions-panel"><div className="panel-title"><span>✦ Sugestões de revisão</span><span>{analysis.length}</span></div><div className="suggestion-list">{analysis.map(item => <div className="suggestion" key={item.id}><div><button className="suggestion-seek" onClick={() => choose(item.media_file_id, item.start_ms)}>{label(item.kind)} · {clock(item.start_ms)}</button><small>{media.find(entry => entry.file.id === item.media_file_id)?.file.original_name} · {Math.round(item.confidence * 100)}%</small></div>{isCut(item) ? <label className="cut-check"><input type="checkbox" aria-label={`Cortar ${label(item.kind)} em ${clock(item.start_ms)}`} disabled={busy} checked={cuts.some(cut => cut.id === item.id)} onChange={() => toggleSuggestion(item)} />Cortar</label> : <span className="suggestion-action">Normalizar áudio</span>}</div>)}{!analysis.length && <p className="empty-media">Importe suas mídias e use Analisar para localizar trechos que precisam de revisão.</p>}</div></section>
      </aside>
      <main className="workspace">
        <section className="viewer-row">
          <div className="monitor-panel panel"><div className="monitor-head"><span>Monitor do clipe original</span><span className="monitor-filename">{selected?.file.original_name ?? 'Prévia'}</span></div>
            <div className="monitor-screen">{selected ? <><video key={contentUrl} ref={player} src={contentUrl} preload="metadata" playsInline aria-label={`Prévia de ${selected.file.original_name}`} onLoadedMetadata={() => { if (player.current) player.current.currentTime = pendingSeek.current / 1000; pendingSeek.current = 0 }} onTimeUpdate={event => setTime(Math.round(event.currentTarget.currentTime * 1000))} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setPlaying(false)} onError={() => setPreviewError('O navegador não consegue reproduzir este formato. O original continua disponível para download.')} />{!selected.metadata?.video_codec && !previewError && <div className="audio-placeholder"><span>♬</span><strong>{selected.file.original_name}</strong><small>Clipe de áudio</small></div>}{previewError && <div className="preview-error" role="alert">{previewError}<a href={contentUrl} target="_blank" rel="noreferrer">Abrir original</a></div>}<span className="timecode">{clock(time)}</span><span className="screen-status">{selected.metadata?.width ? `${selected.metadata.width} × ${selected.metadata.height}` : 'Áudio'}</span></> : <div className="monitor-empty"><span className="empty-symbol">▤</span><h1>Seu próximo vídeo começa aqui</h1><p>Organize mídias, revise trechos e prepare sua edição.</p><button className="outline-button" disabled={busy} onClick={() => { if (project) fileInput.current?.click(); else { setName(''); setProjectForm('new') } }}>{project ? 'Importar mídias' : 'Criar projeto'}</button></div>}</div>
            <div className="transport"><div className="transport-left"><button aria-label="Voltar 5 segundos" disabled={!selected} onClick={() => seek(Math.max(0, time - 5000))}>−5s</button><button className="transport-button play" aria-label={playing ? 'Pausar' : 'Reproduzir'} disabled={!selected || !!previewError} onClick={() => void togglePlayback()}>{playing ? 'Ⅱ' : '▶'}</button><button aria-label="Avançar 5 segundos" disabled={!selected} onClick={() => seek(Math.min(duration, time + 5000))}>+5s</button></div><div className="transport-progress"><input aria-label="Posição no clipe" type="range" min={0} max={duration || 1} step={10} value={Math.min(time, duration)} disabled={!duration} onChange={event => seek(Number(event.target.value))} /><div className="transport-times"><span>{clock(time)}</span><span>{clock(duration)}</span></div></div></div>
          </div>
          <aside className="inspector panel"><div className="panel-title">Clipe e exportação</div><div className="inspector-content">
            <div className="selected-heading"><strong>{selected?.file.original_name ?? 'Nenhum clipe selecionado'}</strong></div>
            <dl className="metadata"><div><dt>Duração</dt><dd>{clock(duration)}</dd></div><div><dt>Vídeo</dt><dd>{selected?.metadata?.video_codec ?? '—'}</dd></div><div><dt>Áudio</dt><dd>{selected?.metadata?.audio_codec ?? '—'}</dd></div><div><dt>Tamanho</dt><dd>{selected ? `${(selected.file.size_bytes / 1024 / 1024).toFixed(1)} MB` : '—'}</dd></div></dl>
            <section className="inspector-section"><h2 className="section-label">Remover intervalo do clipe</h2><div className="cut-fields"><label>Início (s)<input aria-label="Início do corte em segundos" type="number" min={0} max={duration / 1000} step="0.01" disabled={!selected || busy} value={Number.isFinite(cutStart) ? cutStart : ''} onChange={event => setCutStart(event.target.valueAsNumber)} /><button disabled={!selected || busy} onClick={() => setCutStart(time / 1000)}>Marcar início aqui</button></label><label>Fim (s)<input aria-label="Fim do corte em segundos" type="number" min={0} max={duration / 1000} step="0.01" disabled={!selected || busy} value={Number.isFinite(cutEnd) ? cutEnd : ''} onChange={event => setCutEnd(event.target.valueAsNumber)} /><button disabled={!selected || busy} onClick={() => setCutEnd(time / 1000)}>Marcar fim aqui</button></label></div><button className="apply-button" disabled={!selected || busy} onClick={addCut}>✂ Adicionar corte</button><p>O monitor reproduz o original. Os cortes são aplicados na exportação.</p></section>
            <section className="inspector-section"><label className="normalize"><input type="checkbox" checked={normalize} disabled={busy} onChange={event => setNormalize(event.target.checked)} /> Normalizar áudio ao exportar</label>{renderProblem && <p className="limitation">{renderProblem}</p>}</section>
            <section className="inspector-section"><h2 className="section-label">Processamento</h2><p aria-live="polite">{job ? `${job.current_step ?? job.type} · ${job.status} · ${Math.round(job.progress * 100)}%` : 'Nenhum processo nesta sessão.'}</p>{busy && job && <progress aria-label="Progresso do processamento" value={job.progress} max={1} />}</section>
            {renders.some(item => item.status === 'COMPLETED') && <section className="inspector-section"><h2 className="section-label">Exportações concluídas</h2>{renders.filter(item => item.status === 'COMPLETED').map((item, index) => <a className="download-link" key={item.id} href={`${API}/api/projects/${project?.id}/render-plans/${item.id}/download`}>↓ Baixar vídeo {index === 0 ? '(mais recente)' : `· ${item.rendered_at}`}</a>)}</section>}
          </div></aside>
        </section>
        <section className="timeline-panel panel"><div className="timeline-head"><div className="timeline-title"><strong>Sequência</strong><span>{cuts.length} corte(s)</span></div><div className="timeline-actions"><button aria-label="Desfazer corte" disabled={!history.past.length || busy} onClick={undo}>↶ Desfazer</button><button aria-label="Refazer corte" disabled={!history.future.length || busy} onClick={redo}>↷ Refazer</button><label>Zoom <input aria-label="Zoom da sequência" type="range" min={1} max={4} step="0.5" value={zoom} onChange={event => setZoom(Number(event.target.value))} /></label></div></div>
          <div className="sequence-scroll"><div className="sequence-canvas" style={{ width: `${zoom * 100}%` }}><div className="sequence-ruler">{Array.from({ length: 7 }, (_, index) => <span key={index}>{clock(total * index / 6)}</span>)}</div><div className="sequence-track">{media.map(item => <button key={item.file.id} className={`sequence-clip ${selectedId === item.file.id ? 'selected' : ''}`} style={{ width: `${total ? (item.metadata?.duration_ms ?? 0) / total * 100 : 100 / media.length}%` }} title={`${item.file.original_name} · ${clock(item.metadata?.duration_ms)}`} aria-label={`Selecionar na sequência: ${item.file.original_name}`} onClick={event => { const rect = event.currentTarget.getBoundingClientRect(); choose(item.file.id, event.detail === 0 ? 0 : Math.round((event.clientX - rect.left) / rect.width * (item.metadata?.duration_ms ?? 0))) }}><strong>{item.file.original_name}</strong><small>{clock(item.metadata?.duration_ms)}</small>{cuts.filter(cut => cut.media_id === item.file.id).map(cut => <span className="cut-overlay" key={cut.id} style={{ left: `${cut.start_ms / (item.metadata?.duration_ms || 1) * 100}%`, width: `${(cut.end_ms - cut.start_ms) / (item.metadata?.duration_ms || 1) * 100}%` }} />)}</button>)}{total > 0 && <div className="sequence-playhead" style={{ left: `${(offset + time) / total * 100}%` }} />}{!media.length && <p className="timeline-empty">Importe mídias para montar a sequência.</p>}</div></div></div>
          <div className="cut-list">{cuts.map((cut, index) => <div key={cut.id}><button onClick={() => choose(cut.media_id, cut.start_ms)} title={media.find(item => item.file.id === cut.media_id)?.file.original_name}>Corte {index + 1} · {clock(cut.start_ms)}–{clock(cut.end_ms)}</button><button aria-label={`Remover corte ${index + 1}`} disabled={busy} onClick={() => commit(cuts.filter(item => item.id !== cut.id))}>×</button></div>)}{!cuts.length && <span>Selecione uma sugestão ou marque um intervalo para cortar.</span>}</div>
          <div className="timeline-footer"><span>Original {clock(total)} · Cortes {clock(removed)} · Resultado estimado {clock(total - removed)}</span><span>Revisão local · Originais preservados</span></div>
        </section>
      </main>
    </div>
    <footer className="statusbar"><span role="status">{notice}</span><span>{media.length} mídia(s) · {project ? project.name : 'Nenhum projeto aberto'}</span></footer>
  </div>
}
