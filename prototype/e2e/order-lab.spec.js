const {test, expect} = require('@playwright/test');
const {spawn} = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');

const root = path.resolve(__dirname, '..');
let server;
let directory;
let serverOutput = '';
let successRequest;
let successOrder;

async function startServer() {
  server = spawn('mix', ['run', '--no-halt'], {
    cwd: root,
    detached: true,
    env: {...process.env, PORT:'4100', DATABASE_PATH:path.join(directory, 'e2e.sqlite3'),
      MIX_HOME:path.join(root,'.mix'), HEX_HOME:path.join(root,'.hex')},
    stdio: ['ignore','pipe','pipe']
  });
  server.stdout.on('data', d => serverOutput += d.toString());
  server.stderr.on('data', d => serverOutput += d.toString());
  for (let attempt=0; attempt<150; attempt++) {
    if (server.exitCode !== null) throw new Error(`Server exited: ${serverOutput}`);
    try { if ((await fetch('http://127.0.0.1:4100/health')).ok) return; } catch {}
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  throw new Error(`Server did not start: ${serverOutput}`);
}

async function stopServer() {
  if (!server || server.exitCode !== null) return;
  const exited = new Promise(resolve => server.once('exit', resolve));
  process.kill(-server.pid, 'SIGTERM');
  await exited;
}

const payload = (overrides = {}) => ({user_id:'u1',items:[{product_id:'p1',quantity:1}],payment_method:'card',...overrides});
async function snapshot(request) { const r=await request.get('/api/admin'); expect(r.status()).toBe(200); return r.json(); }
async function post(request, body, key) {
  const response = await request.post('/api/orders', {data:body,headers:key ? {'Idempotency-Key':key} : {}});
  return {status:response.status(), body:await response.json()};
}

test.describe.serial('Order Lab · real HTTP, browser and persistent SQLite', () => {
  test.beforeAll(async () => { directory=fs.mkdtempSync(path.join(os.tmpdir(),'order-lab-e2e-')); await startServer(); });
  test.afterAll(async () => {
    await stopServer();
    fs.mkdirSync(path.join(__dirname,'test-results'), {recursive:true});
    fs.writeFileSync(path.join(__dirname,'test-results','server.log'),serverOutput);
    fs.rmSync(directory,{recursive:true,force:true});
  });

  test('browser submits an order and admin shows exact plugin requests and responses', async ({page,request}) => {
    const errors=[]; page.on('pageerror', e=>errors.push(e.message));
    await page.goto('/admin');
    await expect(page.getByRole('heading',{name:'Přehled operací.'})).toBeVisible();
    await page.getByRole('button',{name:'＋ Vytvořit objednávku'}).click();
    await page.locator('[data-product="p1"]').fill('2');
    await expect(page.locator('#cart-total')).toContainText('4');
    const responsePromise=page.waitForResponse(r=>r.url().endsWith('/api/orders') && r.request().method()==='POST');
    await page.getByRole('button',{name:'Odeslat objednávku →'}).click();
    const response=await responsePromise; expect(response.status()).toBe(201);
    const body=await response.json(); successRequest=body.request_id; successOrder=body.order.id;
    expect(body.order.total_cents).toBe(498000);
    expect(body.order.items[0]).toMatchObject({name:'Studio sluchátka',price_cents:249000,quantity:2});
    await expect(page.locator('#order-result')).toContainText('Objednávka commitnuta.');
    await page.getByRole('button',{name:'Prozkoumat požadavek →'}).click();
    await expect(page.locator('#request-dialog')).toBeVisible();
    await expect.poll(async()=> (await snapshot(request)).email_jobs.find(j=>j.order_id===successOrder).state).toBe('sent');
    await expect(page.locator('#request-detail')).toContainText('Email.send_confirmation');
    await expect(page.locator('#request-detail')).toContainText('petra@example.test');
    await expect(page.locator('#request-detail')).toContainText('498000');
    await page.screenshot({path:path.join(__dirname,'test-results','request-detail.png'),fullPage:true});
    const detail=await (await request.get(`/api/requests/${successRequest}`)).json();
    expect(detail.plugin_calls.map(c=>c.plugin)).toEqual(['Payment','Email']);
    expect(detail.plugin_calls[0].input).toMatchObject({amount_cents:498000,country:'CZ',method:'card'});
    expect(detail.plugin_calls[1].output.delivery).toBe('simulated');
    expect((await snapshot(request)).products.find(p=>p.id==='p1').stock).toBe(10);
    expect((await request.get(body.payment.url)).status()).toBe(200);
    expect(errors).toEqual([]);
  });

  test('stock failure rolls back previously decremented products and never calls plugins', async ({request}) => {
    const before=await snapshot(request);
    const result=await post(request,payload({items:[{product_id:'p1',quantity:1},{product_id:'p4',quantity:1}]}));
    expect(result.status).toBe(409); expect(result.body.error.code).toBe('insufficient_stock');
    const after=await snapshot(request);
    expect(after.products).toEqual(before.products); expect(after.orders).toEqual(before.orders);
    expect(after.email_jobs.length).toBe(before.email_jobs.length);
    expect(after.plugin_calls.filter(c=>c.request_id===result.body.request_id)).toEqual([]);
  });

  test('unsupported payment country rolls back order and stock, preserving failed plugin trace', async ({request}) => {
    const before=await snapshot(request);
    const result=await post(request,payload({user_id:'u3'}));
    expect(result.status).toBe(422); expect(result.body.error.code).toBe('payment_country_unsupported');
    const after=await snapshot(request);
    expect(after.products).toEqual(before.products); expect(after.orders).toEqual(before.orders);
    expect(after.email_jobs.length).toBe(before.email_jobs.length);
    const calls=after.plugin_calls.filter(c=>c.request_id===result.body.request_id);
    expect(calls).toHaveLength(1); expect(calls[0]).toMatchObject({plugin:'Payment',status:'error',input:{country:'BR'}});
    expect((await request.get(`/api/orders/${calls[0].order_id}`)).status()).toBe(404);
  });

  test('bank payment country restriction is a distinct business error', async ({request}) => {
    const result=await post(request,payload({user_id:'u2',payment_method:'bank'}));
    expect(result.status).toBe(422); expect(result.body.error.code).toBe('payment_method_unsupported');
  });

  test('typed inputs reject ranges, unknown fields, empty lists and nonexistent identities', async ({request}) => {
    const before=await snapshot(request);
    for (const bad of [payload({items:[]}),payload({items:[{product_id:'p1',quantity:0}]}),payload({items:[{product_id:'p1',quantity:101}]}),payload({items:[{product_id:'p1',quantity:1.5}]}),payload({email_failure:'yes'}),payload({price_cents:1}),payload({payment_method:'bitcoin'})]) {
      const result=await post(request,bad); expect(result.status).toBe(422); expect(result.body.error.code).toBe('invalid_input');
    }
    expect((await post(request,payload({user_id:'missing'}))).status).toBe(404);
    expect((await post(request,payload({items:[{product_id:'missing',quantity:1}]}))).status).toBe(404);
    expect((await request.post('/api/orders',{headers:{'Content-Type':'application/json'},data:Buffer.from('{broken')})).status()).toBe(400);
    const after=await snapshot(request); expect(after.orders).toEqual(before.orders); expect(after.products).toEqual(before.products);
  });

  test('idempotent HTTP replay creates no second order, payment or email', async ({request}) => {
    const body=payload(); const first=await post(request,body,'e2e-replay');
    expect(first.status).toBe(201);
    await expect.poll(async()=> (await snapshot(request)).email_jobs.find(j=>j.order_id===first.body.order.id).state).toBe('sent');
    const before=await snapshot(request);
    const replay=await post(request,body,'e2e-replay');
    expect(replay.status).toBe(201); expect(replay.body.replayed).toBe(true); expect(replay.body.order.id).toBe(first.body.order.id);
    const conflict=await post(request,payload({payment_method:'bank'}),'e2e-replay');
    expect(conflict.status).toBe(409); expect(conflict.body.error.code).toBe('idempotency_conflict');
    const after=await snapshot(request); expect(after.orders).toEqual(before.orders); expect(after.products).toEqual(before.products); expect(after.plugin_calls).toEqual(before.plugin_calls);
  });

  test('concurrent orders and duplicate cart lines cannot oversell inventory', async ({request}) => {
    const results=await Promise.all([post(request,payload({items:[{product_id:'p3',quantity:1},{product_id:'p3',quantity:1}]})),post(request,payload({items:[{product_id:'p3',quantity:2}]}))]);
    expect(results.map(r=>r.status).sort()).toEqual([201,409]);
    const after=await snapshot(request); expect(after.products.find(p=>p.id==='p3').stock).toBe(0);
    const order=results.find(r=>r.status===201).body.order; expect(order.items).toHaveLength(1); expect(order.items[0].quantity).toBe(2); expect(order.total_cents).toBe(258000);
  });

  test('email retries are visible, preserve committed order and can be repaired in admin', async ({page,request}) => {
    const result=await post(request,payload({items:[{product_id:'p2',quantity:1}],email_failure:true}));
    expect(result.status).toBe(201);
    await expect.poll(async()=> (await snapshot(request)).email_jobs.find(j=>j.order_id===result.body.order.id).state,{timeout:10000}).toBe('failed');
    const before=await snapshot(request); const job=before.email_jobs.find(j=>j.order_id===result.body.order.id);
    expect(job.attempts).toBe(3); expect(job.final_disposition).toBe('retain');
    const calls=before.plugin_calls.filter(c=>c.request_id===result.body.request_id);
    expect(calls.filter(c=>c.plugin==='Payment')).toHaveLength(1); expect(calls.filter(c=>c.plugin==='Email')).toHaveLength(3);
    expect((await request.get(`/api/orders/${result.body.order.id}`)).status()).toBe(200);
    await page.goto('/admin#orders');
    const card=page.locator('.order-card').filter({hasText:result.body.order.id});
    await card.getByRole('button',{name:'Opravit a zopakovat'}).click();
    await expect.poll(async()=> (await snapshot(request)).email_jobs.find(j=>j.id===job.id).state).toBe('sent');
    const after=await snapshot(request); expect(after.orders).toEqual(before.orders); expect(after.products).toEqual(before.products);
    expect(after.plugin_calls.filter(c=>c.request_id===result.body.request_id && c.plugin==='Payment')).toHaveLength(1);
  });

  test('admin filters, declarative workflow and mobile layout work in browser', async ({page}) => {
    const errors=[]; page.on('pageerror',e=>errors.push(e.message));
    await page.goto('/admin');
    await page.locator('#request-filter').selectOption('failed');
    await expect(page.locator('#requests-body')).toContainText('Chyba');
    await expect(page.locator('#requests-body')).not.toContainText('Commitnuto');
    await page.locator('#request-filter').selectOption('all');
    await page.locator('#request-search').fill(successRequest);
    await expect(page.locator('#requests-body tr')).toHaveCount(1);
    await page.locator('#request-search').fill('');
    await page.screenshot({path:path.join(__dirname,'test-results','admin-overview.png'),fullPage:true});
    await page.getByRole('button',{name:'Deklarace toku'}).click();
    await expect(page.locator('#workflow-source')).toContainText('CALL Payment.create_url');
    await page.getByRole('button',{name:'Pluginy',exact:false}).click();
    await page.locator('#plugin-filter').selectOption('Payment');
    await expect(page.locator('#plugin-calls')).not.toContainText('Email.send_confirmation');
    await page.setViewportSize({width:390,height:844});
    await page.getByRole('button',{name:'Nová objednávka'}).click();
    await expect(page.getByRole('heading',{name:'Nová objednávka.'})).toBeVisible();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth)).toBe(true);
    await page.screenshot({path:path.join(__dirname,'test-results','mobile-checkout.png'),fullPage:true});
    expect(errors).toEqual([]);
  });

  test('process restart preserves SQL state, request history, plugin calls and completed idempotency keys', async ({request}) => {
    await expect.poll(async()=> (await snapshot(request)).email_jobs.every(j=>['sent','failed'].includes(j.state))).toBe(true);
    const before=await snapshot(request);
    await stopServer(); await startServer();
    const after=await snapshot(request);
    for (const key of ['products','users','orders','requests','plugin_calls','email_jobs']) expect(after[key]).toEqual(before[key]);
    const replay=await post(request,payload(),'e2e-replay'); expect(replay.status).toBe(201); expect(replay.body.replayed).toBe(true);
    expect((await snapshot(request)).orders).toEqual(before.orders);
  });
});
