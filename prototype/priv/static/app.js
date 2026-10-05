const $ = (selector) => document.querySelector(selector);
const esc = (value) => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const pretty = (value) => esc(JSON.stringify(value, null, 2));
const money = (cents) => new Intl.NumberFormat('cs-CZ', {style:'currency', currency:'CZK', maximumFractionDigits:2}).format(cents / 100);
const time = (date) => new Date(date).toLocaleTimeString('cs-CZ');
const short = (id) => id ? `${id.slice(0, 12)}…` : '—';
const labels = {devices:'Živá zařízení',overview:'Přehled',lab:'Nová objednávka',orders:'Objednávky',plugins:'Pluginy',workflow:'Deklarace toku'};
let data = null;
let initialized = false;
let selectedRequest = null;
let refreshing = false;
const badge = (state) => {
  const text = {committed:'Commitnuto',failed:'Chyba',running:'Probíhá',success:'Úspěch',error:'Chyba',sent:'Simulovaně odesláno',queued:'Ve frontě',retrying:'Další pokus',awaiting_payment:'Čeká na platbu'}[state] || state;
  const type = ['failed','error'].includes(state) ? 'error' : ['running','queued','retrying'].includes(state) ? 'pending' : '';
  return `<span class="badge ${type}">${esc(text)}</span>`;
};

async function api(path, options = {}) {
  const response = await fetch(path, options);
  const body = await response.json();
  return {status: response.status, body};
}

function page(name) {
  document.querySelectorAll('.page').forEach(el => el.hidden = el.id !== `page-${name}`);
  document.querySelectorAll('.nav-button').forEach(el => el.classList.toggle('active', el.dataset.page === name));
  $('#breadcrumb').textContent = labels[name];
  location.hash = name;
}

function render() {
  const requests = data.requests;
  const committed = requests.filter(r => r.status === 'committed').length;
  const failed = requests.filter(r => r.status === 'failed').length;
  $('#request-count').textContent = requests.length;
  $('#stats').innerHTML = [
    ['Přijaté požadavky', requests.length, 'posledních 100 požadavků', '⇄'],
    ['Objednávky', data.orders.length, `${committed} úspěšných požadavků`, '▤'],
    ['Chyby požadavků', failed, 'bez nepotvrzených změn', '↶'],
    ['Volání pluginů', data.plugin_calls.length, 'posledních 200 volání', '↗']
  ].map(([label,value,note,icon]) => `<div class="stat"><div class="stat-label">${label}<span class="stat-icon">${icon}</span></div><div class="stat-value">${value}</div><small>${note}</small></div>`).join('');
  $('#updated-at').textContent = `Živě · ${new Date().toLocaleTimeString('cs-CZ')}`;
  renderRequests();
  renderOrders();
  renderPlugins();
  $('#external-operations').innerHTML = (data.external_operations || []).map(operation => `<article class="panel"><div class="panel-title"><h2>${esc(operation.operation)}</h2><span class="tag">${esc(operation.state)} · ${operation.attempts} pokusů</span></div><p>${esc(operation.id)}</p><pre>${pretty(operation.input)}</pre>${operation.state === 'pending' ? (operation.job_id ? (data.email_jobs.find(job => job.id === operation.job_id)?.state === 'failed' ? `<button class="button" data-retry-job="${esc(operation.job_id)}">Zopakovat úlohu</button>` : `<span class="muted">Obnova probíhá automaticky.</span>`) : `<button class="button" data-resume="${esc(operation.request_id)}">Obnovit požadavek</button>`) : ''}<button class="button" data-request="${esc(operation.request_id)}">Původní požadavek →</button></article>`).join('') || '<p class="muted">Zatím žádné externí operace.</p>';
  $('#mqtt-outbox').innerHTML = (data.mqtt_outbox || []).map(job => `<article class="panel"><div class="panel-title"><h2>${esc(job.source)}</h2><span class="tag">${esc(job.state)}</span></div><p>${esc(job.topic)}</p><pre>${pretty(job.payload)}</pre>${job.error ? `<p>${esc(job.error)}</p>` : ''}${job.state === 'failed' ? `<button class="button" data-retry-mqtt="${esc(job.id)}">Zopakovat po opravě kontraktu</button>` : ''}<button class="button" data-request="${esc(job.request_id)}">Původní požadavek →</button></article>`).join('') || '<p class="muted">Zatím žádné odchozí zprávy.</p>';
  $('#workflow-source').textContent = data.workflow;
  if (!initialized) {
    $('#user').innerHTML = data.users.map(u => `<option value="${esc(u.id)}">${esc(u.name)} · ${esc(u.country)}</option>`).join('');
    const icons = ['♬','⌨','⊞','◉'];
    $('#products').innerHTML = data.products.map((p,i) => `<div class="product"><div class="product-icon">${icons[i]}</div><div><h3>${esc(p.name)}</h3><small data-stock="${esc(p.id)}">${p.stock} ks skladem</small><div class="price">${money(p.price_cents)}</div></div><input type="number" min="0" max="100" step="1" value="0" data-product="${esc(p.id)}" aria-label="Počet ${esc(p.name)}"></div>`).join('');
    initialized = true;
  }
  data.products.forEach(p => document.querySelector(`[data-stock="${p.id}"]`).textContent = `${p.stock} ks skladem`);
  updateTotal();
}

function renderRequests() {
  const filter = $('#request-filter').value;
  const search = $('#request-search').value.toLowerCase();
  const requests = data.requests.filter(r => (filter === 'all' || r.status === filter) &&
    `${r.id} ${r.error_code || ''} ${data.users.find(u => u.id === r.input?.user_id)?.name || ''}`.toLowerCase().includes(search));
  $('#requests-empty').hidden = requests.length !== 0;
  $('#requests-body').innerHTML = requests.map(r => {
    const user = data.users.find(u => u.id === r.input?.user_id);
    const calls = data.plugin_calls.filter(c => c.request_id === (r.replayed_from || r.id));
    return `<tr><td class="request-cell"><span class="method">${esc(r.method)}</span>${esc(r.path)}<strong>${esc(short(r.id))}</strong></td><td class="user-cell">${esc(user?.name || 'Neznámý uživatel')}<small>${esc(user?.country || '')}</small></td><td>${badge(r.status)}${r.replayed_from ? '<small class="muted"> · replay</small>' : ''}</td><td>${calls.length ? calls.map(c => c.plugin).filter((v,i,a) => a.indexOf(v) === i).join(' + ') : '<span class="muted">—</span>'}</td><td class="user-cell">${time(r.created_at)}<small>${r.duration_ms ?? '—'} ms</small></td><td><button class="button" data-request="${esc(r.id)}">Detail →</button></td></tr>`;
  }).join('');
}

function renderOrders() {
  $('#orders-list').innerHTML = data.orders.length ? data.orders.map(o => {
    const email = data.email_jobs.find(j => j.order_id === o.id);
    return `<article class="panel order-card"><div class="order-heading"><div><h2>${esc(o.user_name)}</h2><p class="muted">${esc(o.id)} · ${time(o.created_at)}</p></div>${badge(o.status)}</div><ul>${o.items.map(i => `<li><span>${esc(i.name)} × ${i.quantity}</span><span>${money(i.price_cents * i.quantity)}</span></li>`).join('')}</ul><footer><strong>${money(o.total_cents)}</strong><a class="button" href="${esc(o.payment_url)}" target="_blank" rel="noopener">Platební odkaz ↗</a></footer><div class="email-state">Email ${badge(email?.state || 'queued')} · ${email?.attempts || 0} / 3 pokusy ${email?.state === 'failed' ? `<button class="button" data-retry="${esc(email.id)}">Opravit a zopakovat</button>` : ''}</div><div class="email-state"><button class="button" data-request="${esc(o.request_id)}">Zobrazit původní požadavek →</button></div></article>`;
  }).join('') : '<div class="panel empty"><h3>Zatím žádná potvrzená objednávka</h3><p>Neúspěšné pokusy najdete v přehledu požadavků.</p></div>';
}

function callCard(call) {
  return `<article class="call-card" data-call-id="${esc(call.id)}"><div class="call-top"><strong>${esc(call.plugin)}.${esc(call.operation)}</strong>${badge(call.status)}</div><div class="call-meta">${time(call.created_at)} · ${call.duration_ms} ms · pokus ${call.attempt} · ${esc(short(call.order_id))}</div><div class="json-grid"><div><div class="json-title">REQUEST / VSTUP</div><pre>${pretty(call.input)}</pre></div><div><div class="json-title">RESPONSE / VÝSTUP</div><pre>${pretty(call.output)}</pre></div></div></article>`;
}

function renderPlugins() {
  const filter = $('#plugin-filter').value;
  const calls = data.plugin_calls.filter(c => filter === 'all' || c.plugin === filter);
  $('#plugin-calls').innerHTML = calls.length ? calls.map(c => `<div>${callCard(c)}<button class="button" data-request="${esc(c.request_id)}" style="margin-bottom:20px">Původní požadavek →</button></div>`).join('') : '<div class="panel empty"><h3>Zatím žádná volání</h3><p>Pluginy se spustí při zpracování objednávky.</p></div>';
}

function updateTotal() {
  if (!data) return;
  const total = data.products.reduce((sum,p) => sum + p.price_cents * Number(document.querySelector(`[data-product="${p.id}"]`)?.value || 0), 0);
  $('#cart-total').textContent = money(total);
}

async function detail(id, open = true) {
  const {status,body} = await api(`/api/requests/${encodeURIComponent(id)}`);
  if (status !== 200) return;
  const r = body.request;
  $('#detail-title').textContent = r.id;
  $('#request-detail').innerHTML = `<div class="detail-summary"><span class="tag">${esc(r.method)} ${esc(r.path)}</span>${badge(r.status)}<span class="muted">${r.method === 'MQTT' ? 'Výsledek' : 'HTTP'} ${r.http_status} · ${r.duration_ms} ms</span></div>${r.replayed_from ? `<p class="hint">Opakování požadavku ${esc(r.replayed_from)}; pluginy se znovu nevolaly.</p>` : ''}<div class="json-grid"><div><div class="json-title">PŘIJATÝ POŽADAVEK</div><pre>${pretty(r.input)}</pre></div><div><div class="json-title">ODPOVĚĎ ENDPOINTU</div><pre>${pretty(r.response)}</pre></div></div><h2 class="timeline-heading">Volání pluginů <span class="muted">${body.plugin_calls.length} volání</span></h2>${body.plugin_calls.length ? body.plugin_calls.map(callCard).join('') : '<p class="hint">Tento požadavek nevyvolal žádné pluginy.</p>'}`;
  selectedRequest = id;
  if (open && !$('#request-dialog').open) $('#request-dialog').showModal();
}

async function refresh() {
  if (refreshing) return;
  refreshing = true;
  try {
    const result = await api('/api/admin');
    if (result.status !== 200) throw new Error('Runtime unavailable');
    data = result.body;
    $('#connection-error').hidden = true;
    render();
    if ($('#request-dialog').open && selectedRequest) await detail(selectedRequest, false);
  } catch {
    $('#connection-error').hidden = false;
  } finally {
    refreshing = false;
  }
}

document.addEventListener('click', async event => {
  const nav = event.target.closest('[data-page]');
  if (nav) page(nav.dataset.page);
  const request = event.target.closest('[data-request]');
  if (request) await detail(request.dataset.request);
  const resume = event.target.closest('[data-resume]');
  if (resume) {
    resume.disabled = true;
    try {await api(`/api/requests/${encodeURIComponent(resume.dataset.resume)}/resume`, {method:'POST'});await refresh();}
    finally {resume.disabled = false;}
  }
  const jobRetry = event.target.closest('[data-retry-job]');
  if (jobRetry) {
    jobRetry.disabled = true;
    try {await api(`/api/jobs/${encodeURIComponent(jobRetry.dataset.retryJob)}/retry`, {method:'POST'});await refresh();}
    finally {jobRetry.disabled = false;}
  }
  const mqttRetry = event.target.closest('[data-retry-mqtt]');
  if (mqttRetry) {
    mqttRetry.disabled = true;
    try {await api(`/api/mqtt-outbox/${encodeURIComponent(mqttRetry.dataset.retryMqtt)}/retry`, {method:'POST'});await refresh();}
    finally {mqttRetry.disabled = false;}
  }
  const retry = event.target.closest('[data-retry]');
  if (retry) {
    retry.disabled = true;
    try { await api(`/api/email-jobs/${encodeURIComponent(retry.dataset.retry)}/retry`, {method:'POST'}); await refresh(); }
    finally { retry.disabled = false; }
  }
});
$('#close-dialog').addEventListener('click', () => $('#request-dialog').close());
$('#request-dialog').addEventListener('click', event => { if (event.target === $('#request-dialog')) { const rect = event.target.getBoundingClientRect(); if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) event.target.close(); } });
$('#request-search').addEventListener('input', () => data && renderRequests());
$('#request-filter').addEventListener('change', () => data && renderRequests());
$('#plugin-filter').addEventListener('change', () => data && renderPlugins());
$('#products').addEventListener('input', updateTotal);
$('#order-form').addEventListener('submit', async event => {
  event.preventDefault();
  const items = [...document.querySelectorAll('[data-product]')].filter(el => Number(el.value) > 0).map(el => ({product_id:el.dataset.product, quantity:Number(el.value)}));
  const result = $('#order-result');
  const button = $('#submit-order');
  if (!items.length) { result.hidden = false; result.className = 'panel result failure'; result.textContent = 'Vyberte alespoň jeden produkt.'; return; }
  button.disabled = true; button.textContent = 'Zpracovávám…';
  try {
    const {status,body} = await api('/api/orders', {method:'POST',headers:{'Content-Type':'application/json','Idempotency-Key':crypto.randomUUID()},body:JSON.stringify({user_id:$('#user').value,items,payment_method:$('#payment').value,email_failure:$('#email-failure').checked})});
    const ok = status < 400;
    result.hidden = false; result.className = `panel result ${ok ? 'success' : 'failure'}`;
    result.innerHTML = `<h3>${ok ? 'Objednávka commitnuta.' : 'Objednávka nevznikla.'}</h3><p>${ok ? 'Sklad byl odečten a email čeká ve frontě.' : esc(body.error?.message || 'Požadavek selhal.')}</p><span class="tag">HTTP ${status}</span>${body.request_id ? `<button class="button" data-request="${esc(body.request_id)}">Prozkoumat požadavek →</button>` : ''}${ok ? `<a class="button" href="${esc(body.payment.url)}" target="_blank" rel="noopener">Platba ↗</a>` : ''}`;
    if (ok) document.querySelectorAll('[data-product]').forEach(el => el.value = 0);
    await refresh();
  } catch { result.hidden = false; result.className = 'panel result failure'; result.textContent = 'Spojení se přerušilo; výsledek ověřte v přehledu.'; }
  finally { button.disabled = false; button.textContent = 'Odeslat objednávku →'; }
});
page(labels[location.hash.slice(1)] ? location.hash.slice(1) : 'overview');
refresh();
setInterval(refresh, 1000);

let deviceSocket = null;
let deviceCopies = [];
$('#device-form').addEventListener('submit', event => {
  event.preventDefault();
  if (deviceSocket) deviceSocket.close();
  deviceCopies = [];
  $('#device-messages').textContent = '';
  $('#device-status').textContent = $('#device-position').textContent = 'Čekám na zprávu…';
  const deviceId = $('#device-id').value.trim();
  const socket = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/ws`);
  deviceSocket = socket;
  $('#device-connection').textContent = 'Připojuji…';
  socket.onopen = () => {
    if (deviceSocket !== socket) return;
    $('#device-connection').textContent = 'Připojeno';
    socket.send(JSON.stringify({action:'authenticate', input:{token:$('#device-token').value}}));
  };
  socket.onmessage = event => {
    if (deviceSocket !== socket) return;
    const copy = JSON.parse(event.data);
    if (copy.type === 'authenticated') {
      $('#device-connection').textContent = 'Ověřeno';
      for (const source of ['DeviceStatus', 'DevicePosition']) socket.send(JSON.stringify({action:'subscribe',source,params:{device_id:deviceId},latest:true}));
    }
    if (copy.type === 'error') $('#device-connection').textContent = copy.message;
    if (copy.type !== 'message') return;
    const target = copy.source === 'DeviceStatus' ? '#device-status' : '#device-position';
    $(target).textContent = JSON.stringify(copy.payload, null, 2);
    deviceCopies.unshift(copy); deviceCopies.length = Math.min(deviceCopies.length, 50);
    $('#device-messages').textContent = deviceCopies.map(copy => JSON.stringify(copy)).join('\n');
  };
  socket.onclose = () => { if (deviceSocket === socket) $('#device-connection').textContent = 'Odpojeno'; };
  socket.onerror = () => { if (deviceSocket === socket) $('#device-connection').textContent = 'Chyba spojení'; };
});
$('#device-stop').addEventListener('click', () => { if (deviceSocket) deviceSocket.close(); });

for (const button of document.querySelectorAll('[data-mower-action]')) {
  button.addEventListener('click', async () => {
    button.disabled = true;
    try {
      const {status,body} = await api(`/api/devices/${encodeURIComponent($('#device-id').value.trim())}/commands`, {method:'POST',headers:{'Content-Type':'application/json','Idempotency-Key':crypto.randomUUID()},body:JSON.stringify({action:button.dataset.mowerAction,token:$('#device-token').value})});
      $('#device-command-result').textContent = `HTTP ${status}\n${JSON.stringify(body,null,2)}`;
      await refresh();
    } catch { $('#device-command-result').textContent = 'Spojení se přerušilo; výsledek ověřte v přehledu požadavků.'; }
    finally {button.disabled = false;}
  });
}
