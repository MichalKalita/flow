const {test, expect} = require('@playwright/test');
const {spawn,execFileSync} = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');

const root = path.resolve(__dirname, '..');
let server;
let directory;
let serverOutput = '';
let successRequest;
let successOrder;
let savedPhoto;

async function startServer() {
  server = spawn('mix', ['run', '--no-halt'], {
    cwd: root,
    detached: true,
    env: {...process.env, PORT:'4100', MQTT_PORT:'18830', FLOW_PATH:path.join(directory,'application.flow'), DATABASE_PATH:path.join(directory, 'e2e.sqlite3'),
      ...(fs.existsSync(path.join(root,'.mix')) ? {MIX_HOME:path.join(root,'.mix')} : {}),
      ...(fs.existsSync(path.join(root,'.hex')) ? {HEX_HOME:path.join(root,'.hex')} : {})},
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


const net = require('node:net');
const mqttString = value => {const b=Buffer.from(value); const h=Buffer.alloc(2); h.writeUInt16BE(b.length); return Buffer.concat([h,b]);};
function mqttPacket(header, body=Buffer.alloc(0)) {
  let n=body.length; const size=[];
  do {let digit=n%128; n=Math.floor(n/128); if(n)digit|=128; size.push(digit);} while(n);
  return Buffer.concat([Buffer.from([header,...size]),body]);
}
async function mqttClient() {
  const socket=net.createConnection({host:'127.0.0.1',port:18830});
  socket.on('error',()=>{});
  await new Promise((resolve,reject)=>{socket.once('connect',resolve);socket.once('error',reject);});
  let buffer=Buffer.alloc(0); const packets=[]; const waits=[];
  socket.on('data',data=>{
    buffer=Buffer.concat([buffer,data]);
    while(buffer.length>=2){
      let n=0,m=1,i=1,d;
      do {if(i>=buffer.length)return;d=buffer[i++];n+=(d&127)*m;m*=128;}while(d&128);
      if(buffer.length<i+n)return;
      const packet={header:buffer[0],body:buffer.subarray(i,i+n)};buffer=buffer.subarray(i+n);
      if(waits.length)waits.shift()(packet);else packets.push(packet);
    }
  });
  const next=()=>new Promise((resolve,reject)=>{
    if(packets.length)return resolve(packets.shift());
    const timeout=setTimeout(()=>reject(new Error('MQTT packet timeout')),3000);
    waits.push(p=>{clearTimeout(timeout);resolve(p);});
  });
  socket.write(mqttPacket(0x10,Buffer.concat([mqttString('MQTT'),Buffer.from([4,2,0,60]),mqttString(`e2e-${Math.random()}`)])));
  expect(await next()).toEqual({header:0x20,body:Buffer.from([0,0])});
  return {
    socket,next,
    async publish(topic,value,{retain=false,id=1,dup=false}={}) {
      const identifier=Buffer.alloc(2);identifier.writeUInt16BE(id);
      socket.write(mqttPacket(0x32|(retain?1:0)|(dup?8:0),Buffer.concat([mqttString(topic),identifier,Buffer.from(JSON.stringify(value))])));
      const ack=await next();expect(ack.header).toBe(0x40);expect(ack.body.readUInt16BE()).toBe(id);
    },
    close(){socket.end(mqttPacket(0xE0));}
  };
}
async function wsClient(token='demo-petra') {
  const socket=new WebSocket('ws://127.0.0.1:4100/ws');
  const messages=[];const waits=[];
  socket.addEventListener('message',event=>{const data=JSON.parse(event.data);if(waits.length)waits.shift()(data);else messages.push(data);});
  await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
  const next=()=>new Promise((resolve,reject)=>{if(messages.length)return resolve(messages.shift());const timer=setTimeout(()=>reject(new Error('WebSocket message timeout')),3000);waits.push(value=>{clearTimeout(timer);resolve(value);});});
  const client={socket,next,send(value){socket.send(JSON.stringify(value));},close(){socket.close();}};
  if(token !== null){client.send({action:'authenticate',input:{token}});expect((await client.next()).type).toBe('authenticated');}
  return client;
}
const pngFixture=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAQAAAACCAYAAAB/qH1jAAAAEklEQVR4nGP4z8DwHxkzoAsAAA8hD/EEN8afAAAAAElFTkSuQmCC','base64');
const upload=(data=pngFixture,extra={})=>({data:data.toString('base64'),name:'fixture.png',...extra});
const accessFixtures=['browser-mower','panel-mower','corrupt',...Array.from({length:17},(_,i)=>'limit-'+i)].map(device_id=>({id:'e2e-'+device_id,user_id:'u1',token:'demo-petra',device_id}));
const seedFixtures='\nSEED DeviceAccess WITH '+JSON.stringify(accessFixtures).replace(/"(id|user_id|token|device_id)":/g,'$1:')+'\n';
const extraFlow = seedFixtures+`
HTTP DELETE /api/test-device-access/:id
INPUT id String
TRANSACTION
    DELETE FROM DeviceAccess AS access WHERE access.id = :id
    COMMIT
RETURN {deleted: true}

HTTP POST /api/test-device-access
INPUT id String
INPUT user_id UserID
INPUT token String
INPUT device_id DeviceID
TRANSACTION
    grant = INSERT DeviceAccess WITH {id: :id, user_id: :user_id, token: :token, device_id: :device_id}
    COMMIT
RETURN grant

TYPE Rating = Number WHERE value BETWEEN 1.0 AND 5.0
TYPE Note = {id: String, text: String, rating: Rating, tags: List<String>}
TABLE Notes = Note
FILTER NoteFilter
    minimum Rating?
    tag String?
TABLE Reviews = {id: String, product_id: String, user_id: String, rating: Rating, text: String}
HTTP POST /api/reviews
INPUT product_id String
INPUT user_id String
INPUT rating Rating
INPUT text String
TRANSACTION
    review = INSERT Reviews WITH {id: uuid("review"), product_id: :product_id, user_id: :user_id, rating: :rating, text: :text}
    COMMIT
RESPONSE 201 WITH review
HTTP GET /api/catalog
INPUT show_reviews Bool = false
rows = FROM Products AS p
    WHEN :show_reviews
        LEFT JOIN Reviews AS r ON r.product_id = p.id
    LEFT JOIN Users AS u ON r IS PRESENT AND u.id = r.user_id
    ORDER BY p.id ASC
    SELECT {id: p.id, review: r, author: u, author_name: IF u IS PRESENT THEN u.name ELSE null}
RETURN {rows: rows}
HTTP GET /api/reviews
INPUT minimum Rating = 1
rows = FROM Reviews AS r JOIN Products AS p ON p.id = r.product_id JOIN Users AS u ON u.id = r.user_id
    WHERE r.rating >= :minimum
    ORDER BY r.rating DESC
    SELECT {product: p.name, author: u.name, rating: r.rating}
RETURN {rows: rows}
HTTP GET /api/order-items/:id
INPUT id String
rows = FROM Orders AS o
    JOIN o.items AS item ON true
    WHERE o.id = :id
    SELECT {order_id: o.id, product_id: item.id, quantity: item.quantity}
RETURN {rows: rows}
HTTP POST /api/ratings/constrained
INPUT rating Rating
INPUT delta Number = 0
value: Rating = :rating + :delta
RETURN {rating: value}
HTTP POST /api/ratings/each
INPUT ratings List<Number>
values: List<Rating> = FOR EACH number IN :ratings
    value: Rating = number
    RETURN value
RETURN {ratings: values}
HTTP POST /api/optional-branch
INPUT record {name: String}?
WHEN :record IS NOT PRESENT
    RETURN {name: "anonymous"}
ELSE
    RETURN {name: :record.name}
HTTP POST /api/optional-or
INPUT record {name: String}?
valid = :record IS NOT PRESENT OR :record.name = "valid"
RETURN {valid: valid}
HTTP POST /api/branch-values
INPUT numbers List<Int>
values = FOR EACH number IN :numbers
    WHEN number > 0
        half = number / 2
        RETURN {value: half}
    ELSE
        original = number
        RETURN {value: original}
safe: Number = IF COUNT :numbers > 0 THEN (IF :numbers = [0] THEN 0 ELSE 1) ELSE (IF false THEN 1 / 0 ELSE 0)
RETURN {values: values, safe: safe}
HTTP POST /api/ratings/rollback
INPUT rating Rating
TRANSACTION
    INSERT Notes WITH {id: "typed-rollback", text: "must not persist", rating: 1, tags: []}
    value: Rating = :rating + 1
    COMMIT
RETURN {rating: value}
HTTP POST /api/files/rollback
INPUT file File
TRANSACTION
    saved = CALL Files.put WITH {file: :file}
    INSERT Photos WITH {id: "rolled-back-file", product_id: "p1", file_id: saved.id, url: saved.url, width: 1, height: 1}
    FAIL 409 deliberate_failure "Roll back file and row"
    COMMIT
RETURN true
HTTP DELETE /api/files/:id
INPUT id String
TRANSACTION
    deleted = CALL Files.delete WITH {id: :id}
    COMMIT
RETURN deleted
HTTP POST /api/files/decode
INPUT file File
decoded: Image = CALL Image.decode WITH {file: :file}
RETURN {width: decoded.width, height: decoded.height, format: decoded.format}
HTTP POST /api/notes
INPUT text String
INPUT rating Rating
INPUT tags List<String>
TRANSACTION
    note = INSERT Notes WITH {id: uuid("note"), text: upper(:text), rating: :rating, tags: distinct(:tags)}
    COMMIT
RESPONSE 201 WITH note
HTTP GET /api/notes
INPUT filter NoteFilter
rows = FROM Notes AS n
    WHEN :filter.minimum IS PRESENT
        WHERE n.rating >= :filter.minimum
    WHEN :filter.tag IS PRESENT
        WHERE EXISTS tag IN n.tags WHERE tag = :filter.tag
    ORDER BY n.rating DESC
    SELECT {id: n.id, text: n.text, rating: n.rating}
RETURN {rows: rows, total: COUNT rows, any: EXISTS n IN rows WHERE n.rating >= 4}
HTTP PATCH /api/notes/:id
INPUT id String
INPUT text String
TRANSACTION
    changed = UPDATE Notes AS n WHERE n.id = :id SET {text: :text}
    REQUIRE COUNT changed = 1 ELSE 404 not_found "Missing note"
    COMMIT
RETURN {changed: changed}
HTTP DELETE /api/notes/:id
INPUT id String
TRANSACTION
    removed = DELETE FROM Notes AS n WHERE n.id = :id
    COMMIT
RETURN {removed: removed}
MQTT DeviceAction
    TOPIC "devices/{device_id}/action"
    PARAM device_id String
    PAYLOAD {accept: Bool}
TABLE Events = {id: String, device_id: String}
ON MQTT DeviceAction
    TRANSACTION
        INSERT Events WITH {id: uuid("event"), device_id: :device_id}
        REQUIRE :message.accept ELSE 409 action_rejected "Rejected action"
        COMMIT
    RETURN {accepted: true}
HTTP GET /api/events
rows = FROM Events AS e
RETURN {rows: rows}
HTTP POST /api/jobs/drop
TRANSACTION
    job = QUEUE Email.send_confirmation WITH {
        order_id: "independent", to: "demo@example.test", subject: "test", payment_url: "",
        total_cents: 100, items: [], simulate_failure: true
    } POLICY 1 ATTEMPTS DELAY 0 ms DELETE
    COMMIT
RETURN job
HTTP POST /api/notes/savepoint
TRANSACTION
    TRY
        INSERT Notes WITH {id: "rolled-back", text: "transient", rating: 5, tags: []}
        FAIL 409 temporary_failure "Business error"
    CATCH error
        REQUIRE error.code = "temporary_failure" ELSE 500 wrong_error "Wrong error"
    surviving = INSERT Notes WITH {id: "surviving", text: "kept", rating: 3, tags: []}
    COMMIT
RETURN surviving
`;
test.describe.serial('Order Lab · real HTTP, browser and persistent SQLite', () => {
  test.beforeAll(async () => { directory=fs.mkdtempSync(path.join(os.tmpdir(),'order-lab-e2e-')); fs.writeFileSync(path.join(directory,'application.flow'),fs.readFileSync(path.join(root,'priv/workflows/application.flow'),'utf8')+extraFlow); await startServer(); });
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
    const malformed=await request.post('/api/orders',{headers:{'Content-Type':'application/json'},data:Buffer.from('{broken')});
    expect(malformed.status()).toBe(400);
    const malformedBody=await malformed.json();
    const malformedDetail=await (await request.get(`/api/requests/${malformedBody.request_id}`)).json();
    expect(malformedDetail.request.input.raw_body).toBe('{broken');
    const wrongType=await request.post('/api/orders',{headers:{'Content-Type':'text/plain'},data:Buffer.from('hello')});
    expect(wrongType.status()).toBe(415);
    expect((await wrongType.json()).request_id).toBeTruthy();
    const tooLarge=await request.post('/api/orders',{headers:{'Content-Type':'application/json'},data:Buffer.from('x'.repeat(70000))});
    expect(tooLarge.status()).toBe(413);
    const tooLargeBody=await tooLarge.json();
    expect((await (await request.get(`/api/requests/${tooLargeBody.request_id}`)).json()).request.input.truncated).toBe(true);
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


  test('one source file defines unrelated CRUD, refined inputs, conditional queries, EXISTS and savepoints', async ({request}) => {
    for (const rating of [0,5.1,"4",null]) {
      const response=await request.post('/api/notes',{data:{text:'invalid',rating,tags:[]}});
      expect(response.status()).toBe(422);
    }
    const created=[];
    for (const data of [{text:'first',rating:4.5,tags:['red','red']},{text:'second',rating:2,tags:['blue']}]) {
      const response=await request.post('/api/notes',{data});expect(response.status()).toBe(201);
      created.push(await response.json());
    }
    expect(created[0].text).toBe('FIRST');expect(created[0].tags).toEqual(['red']);
    let response=await request.get('/api/notes',{params:{filter:JSON.stringify({minimum:4,tag:'red'})}});
    expect(response.status()).toBe(200);let result=await response.json();
    expect(result.rows.map(n=>n.id)).toEqual([created[0].id]);expect(result.total).toBe(1);expect(result.any).toBe(true);
    response=await request.get('/api/notes',{params:{filter:'{}'}});expect(response.status()).toBe(200);
    expect((await response.json()).total).toBe(2);
    response=await request.patch(`/api/notes/${created[0].id}`,{data:{text:'changed'}});expect(response.status()).toBe(200);
    expect((await response.json()).changed[0].text).toBe('changed');
    response=await request.post('/api/notes/savepoint',{data:{}});expect(response.status()).toBe(200);
    response=await request.get('/api/notes',{params:{filter:'{}'}});
    result=await response.json();expect(result.rows.map(n=>n.id)).toContain('surviving');expect(result.rows.map(n=>n.id)).not.toContain('rolled-back');
    response=await request.delete(`/api/notes/${created[1].id}`);expect(response.status()).toBe(200);
    expect((await response.json()).removed[0].id).toBe(created[1].id);
    const trace=(await snapshot(request)).requests.find(r=>r.path===`/api/notes/${created[1].id}` && r.method==='DELETE');expect(trace.http_status).toBe(200);
  });



  test('joined queries support conditional LEFT joins, multiple INNER joins and correlated collections', async ({request}) => {
    for(const data of [
      {product_id:'p1',user_id:'u1',rating:5,text:'first'},
      {product_id:'p1',user_id:'u2',rating:4,text:'second'},
      {product_id:'p2',user_id:'u1',rating:3.5,text:'third'}
    ])expect((await request.post('/api/reviews',{data})).status()).toBe(201);
    let response=await request.get('/api/catalog',{params:{show_reviews:'false'}});expect(response.status()).toBe(200);
    let result=await response.json();expect(result.rows).toHaveLength(4);expect(result.rows.every(row=>row.review===null && row.author===null)).toBe(true);expect(result.rows.every(row=>row.author_name===null)).toBe(true);
    response=await request.get('/api/catalog',{params:{show_reviews:'true'}});expect(response.status()).toBe(200);result=await response.json();
    expect(result.rows).toHaveLength(5);expect(result.rows.filter(row=>row.id==='p1').map(row=>row.author.id)).toEqual(['u1','u2']);
    expect(result.rows.find(row=>row.id==='p4').review).toBeNull();expect(result.rows.filter(row=>row.id==='p1').map(row=>row.author_name)).toEqual(['Petra Nováková','David Miller']);
    response=await request.get('/api/reviews',{params:{minimum:'4'}});expect(response.status()).toBe(200);result=await response.json();
    expect(result.rows.map(row=>row.rating)).toEqual([5,4]);expect(result.rows.map(row=>row.author)).toEqual(['Petra Nováková','David Miller']);
    response=await request.get(`/api/order-items/${successOrder}`);expect(response.status()).toBe(200);result=await response.json();
    expect(result.rows).toEqual([{order_id:successOrder,product_id:'p1',quantity:2}]);
  });

  test('typed bindings check computed refinements, bound iterations and transactional rollback', async ({request}) => {
    let response=await request.post('/api/ratings/constrained',{data:{rating:4,delta:0.5}});expect(response.status()).toBe(200);expect((await response.json()).rating).toBe(4.5);
    response=await request.post('/api/ratings/constrained',{data:{rating:5,delta:1}});expect(response.status()).toBe(422);expect((await response.json()).error.code).toBe('type_constraint_failed');
    response=await request.post('/api/ratings/each',{data:{ratings:[1,4.5,5]}});expect(response.status()).toBe(200);expect((await response.json()).ratings).toEqual([1,4.5,5]);
    response=await request.post('/api/ratings/each',{data:{ratings:[1,6]}});expect(response.status()).toBe(422);
    response=await request.post('/api/ratings/rollback',{data:{rating:5}});expect(response.status()).toBe(422);
    response=await request.get('/api/notes',{params:{filter:'{}'}});expect((await response.json()).rows.map(row=>row.id)).not.toContain('typed-rollback');
  });


  test('iteration branches infer local result types and conditional expressions evaluate only the selected branch', async ({request}) => {
    let response=await request.post('/api/branch-values',{data:{numbers:[2,-1,3]}});expect(response.status()).toBe(200);let result=await response.json();
    expect(result.values).toEqual([{value:1},{value:-1},{value:1.5}]);expect(result.safe).toBe(1);
    response=await request.post('/api/branch-values',{data:{numbers:[]}});expect(response.status()).toBe(200);result=await response.json();expect(result.values).toEqual([]);expect(result.safe).toBe(0);
    for(const record of [null,{name:'valid'}]){response=await request.post('/api/optional-branch',{data:{record}});expect(response.status()).toBe(200);expect((await response.json()).name).toBe(record?.name || 'anonymous');response=await request.post('/api/optional-or',{data:{record}});expect(response.status()).toBe(200);expect((await response.json()).valid).toBe(true);}
  });

  test('idempotency and rejected request tracing work for arbitrary Flow routes; queue DELETE retains call diagnostics', async ({request}) => {
    const data={text:'repeatable',rating:4,tags:[]};const headers={'Idempotency-Key':'generic-note'};
    const first=await request.post('/api/notes',{data,headers});expect(first.status()).toBe(201);const note=await first.json();
    const replay=await request.post('/api/notes',{data,headers});expect(replay.status()).toBe(201);expect((await replay.json()).id).toBe(note.id);
    const conflict=await request.post('/api/notes',{data:{...data,text:'different'},headers});expect(conflict.status()).toBe(409);
    const malformed=await request.post('/api/notes',{data:Buffer.from('{'),headers:{'Content-Type':'application/json'}});expect(malformed.status()).toBe(400);const failure=await malformed.json();
    const detail=await request.get(`/api/requests/${failure.request_id}`);const trace=await detail.json();expect(trace.request.path).toBe('/api/notes');expect(trace.request.method).toBe('POST');
    const response=await request.post('/api/jobs/drop',{data:{}});expect(response.status()).toBe(200);const job=await response.json();
    await expect.poll(async()=> (await snapshot(request)).email_jobs.some(j=>j.id===job.id)).toBe(false);
    const calls=(await snapshot(request)).plugin_calls.filter(c=>c.request_id===job.request_id);expect(calls).toHaveLength(1);expect(calls[0].status).toBe('error');
  });

  test('HTTP compiler diagnostics reject unknown fields, optional access, effects and ambiguous MQTT contracts', async ({request}) => {
    const valid='TYPE Rating = Number WHERE value BETWEEN 1 AND 5\nHTTP POST /rating\nINPUT rating Rating\nRETURN {rating: :rating}';
    expect((await request.post('/api/language/check',{data:valid,headers:{'Content-Type':'text/plain'}})).status()).toBe(200);
    const invalid=[
      'REST GET /old\nRETURN true',
      'HTTP POST /bad\nINPUT file File\nCALL Image.resize WITH {image: :file, width: 2, height: 2}\nRETURN true',
      'HTTP POST /bad\nINPUT file File\nCALL Files.put WITH {file: :file}\nRETURN true',
      'WEBSOCKET /ws\n    SOURCE Missing',
      'WEBSOCKET /ws\n    AUTHORIZE "yes"\n    SOURCE DeviceStatus',
      'WEBSOCKET /ws\n    SOURCE DeviceStatus WHERE :device_id > 1',
      'TABLE T = {id: String, rating: Int}\nSEED T WITH [{id: "x", rating: "bad"}]',
      'TABLE T = {id: String}\nSEED T WITH [{id: "x"}]\nSEED T WITH [{id: "x"}]',

      'TYPE Random = String WHERE uuid("id") = value\nHTTP GET /bad\nRETURN true',
      'TYPE Rating = Number WHERE value BETWEEN 1 AND 5\nHTTP GET /bad\nrating: Rating = 7\nRETURN rating',
      'TYPE Rating = Number WHERE value BETWEEN 1 AND 5\nHTTP POST /bad\nINPUT rating Rating = 7\nRETURN :rating',
      'TYPE Rating = Number WHERE value BETWEEN 1 AND 5\nTABLE Ratings = {id: String, rating: Rating}\nHTTP POST /bad\nINPUT id String\nTRANSACTION\n    INSERT Ratings WITH {id: :id, rating: 7}\n    COMMIT\nRETURN true',
      'HTTP GET /bad\nnumbers: List<Int> = [1, 2.5]\nRETURN numbers',
      'TABLE A = {id: String}\nHTTP GET /bad\nrows = FROM A AS a LEFT JOIN A AS b ON b.id = a.id SELECT b.id\nRETURN rows',
      'TABLE A = {id: String}\nHTTP GET /bad\nrows = FROM A AS a JOIN A AS a ON true\nRETURN rows',

      'HTTP GET /bad\nRETURN missing',
      'HTTP POST /bad\nINPUT item {rating: Number}\nRETURN :item.price',
      'HTTP POST /bad\nINPUT item {rating: Number}?\nRETURN :item.rating',
      'HTTP POST /bad\nINSERT Notes WITH {id: "x"}',
      'HTTP POST /bad\nCALL Unknown.send WITH {}\nRETURN true',
      'TYPE X = Missing\nHTTP GET /bad\nRETURN true',
      'TYPE X = X\nHTTP GET /bad\nRETURN true',
      'TYPE X = Number WHERE value + 1\nHTTP GET /bad\nRETURN true',
      'MQTT X\n    TOPIC "a/{id}"\n    PAYLOAD {ok: Bool}',
      'MQTT X\n    TOPIC "a/{id}"\n    PARAM id String\n    PAYLOAD {ok: Bool}\nMQTT Y\n    TOPIC "a/{other}"\n    PARAM other String\n    PAYLOAD {ok: Bool}'
    ];
    for(const source of invalid){const response=await request.post('/api/language/check',{data:source,headers:{'Content-Type':'text/plain'}});expect(response.status(),source).toBe(422);expect((await response.json()).valid).toBe(false);}
  });

  test('real MQTT QoS1 publishes feed typed HTTP sources, history, duplicate suppression and retained subscriptions', async ({request,page}) => {
    const client=await mqttClient();
    try {
      await client.publish('devices/d1/status',{online:true,battery:87},{retain:true});
      await client.publish('devices/d1/position',{latitude:50.1,longitude:14.4},{id:2});
      await client.publish('devices/d1/position',{latitude:50.2,longitude:14.5},{id:3});
      await client.publish('devices/d1/position',{latitude:50.2,longitude:14.5},{id:3,dup:true});
      await client.publish('devices/d2/status',{online:false,battery:10},{id:4});
      const alerts=await request.get('/api/device-alerts/d2');expect((await alerts.json()).alerts).toHaveLength(1);
      const noAlerts=await request.get('/api/device-alerts/d1');expect((await noAlerts.json()).alerts).toEqual([]);
      const event=(await snapshot(request)).requests.find(r=>r.method==='MQTT' && r.path==='devices/d2/status');expect(event.http_status).toBe(200);expect(event.input.message.battery).toBe(10);
      await page.goto('/admin');await page.locator('#request-search').fill(event.id);await expect(page.locator('#requests-body')).toContainText('MQTT');await expect(page.locator('#requests-body')).toContainText('devices/d2/status');
      let response=await request.get('/api/devices/d1');expect(response.status()).toBe(200);
      const device=await response.json();expect(device.status.online).toBe(true);expect(device.status.battery).toBe(87);
      expect(device.positions.map(p=>p.latitude)).toEqual([50.1,50.2]);expect(device.positions.every(p=>p.received_at)).toBe(true);
      const subscriber=await mqttClient();
      try {
        subscriber.socket.write(mqttPacket(0x82,Buffer.concat([Buffer.from([0,1]),mqttString('devices/+/status'),Buffer.from([0])])));
        expect((await subscriber.next()).header).toBe(0x90);
        const retained=await subscriber.next();expect(retained.header).toBe(0x31);
        const length=retained.body.readUInt16BE();expect(retained.body.subarray(2,2+length).toString()).toBe('devices/d1/status');
        expect(JSON.parse(retained.body.subarray(2+length))).toEqual({online:true,battery:87});
        await client.publish('devices/d1/status',{online:false,battery:80},{id:5});
        const live=await subscriber.next();expect(live.header).toBe(0x30);
        response=await request.get('/api/devices/d1');expect((await response.json()).status.battery).toBe(80);
      } finally {subscriber.close();}
    } finally {client.close();}
    const empty=await request.get('/api/devices/unknown');const value=await empty.json();expect(value.status).toBeNull();expect(value.positions).toEqual([]);
  });

  test('MQTT rejects out-of-range payloads and unknown fields before they enter typed history', async ({request}) => {
    for(const payload of [{latitude:91,longitude:14},{latitude:50,longitude:181},{latitude:"50",longitude:14},{latitude:50,longitude:14,extra:true}]) {
      const client=await mqttClient();
      const closed=new Promise(resolve=>client.socket.once('close',resolve));
      client.socket.write(mqttPacket(0x32,Buffer.concat([mqttString('devices/rejected/position'),Buffer.from([0,1]),Buffer.from(JSON.stringify(payload))])));
      await closed;
    }
    const response=await request.get('/api/devices/rejected');expect(response.status()).toBe(200);expect((await response.json()).positions).toEqual([]);
  });


  test('ON MQTT business failures roll back table writes while accepting and tracing the incoming message', async ({request}) => {
    const client=await mqttClient();
    try {
      await client.publish('devices/d3/action',{accept:false});
      await client.publish('devices/d4/action',{accept:true},{id:2});
    } finally {client.close();}
    const events=await request.get('/api/events');expect((await events.json()).rows.map(e=>e.device_id)).toEqual(['d4']);
    const traces=(await snapshot(request)).requests.filter(r=>r.method==='MQTT');
    const failed=traces.find(r=>r.path==='devices/d3/action');expect(failed.http_status).toBe(409);expect(failed.error_code).toBe('action_rejected');
    expect(traces.find(r=>r.path==='devices/d4/action').http_status).toBe(200);
  });


  test('real PNG resize and native file storage commit together and Chrome decodes the resulting pixels', async ({request,page}) => {
    const response=await request.post('/api/products/p1/photo',{data:{photo:upload(),width:2,height:2},headers:{'Idempotency-Key':'photo-1'}});expect(response.status()).toBe(201);savedPhoto=await response.json();
    expect(savedPhoto.photo.width).toBe(2);expect(savedPhoto.photo.height).toBe(1);expect(savedPhoto.file.media_type).toBe('image/png');
    const download=await request.get(savedPhoto.file.url);expect(download.status()).toBe(200);const bytes=await download.body();expect(bytes.readUInt32BE(16)).toBe(2);expect(bytes.readUInt32BE(20)).toBe(1);
    await page.goto('/admin');
    const decoded=await page.evaluate(async url=>{const image=new Image();image.src=url;await image.decode();const canvas=document.createElement('canvas');canvas.width=image.width;canvas.height=image.height;canvas.getContext('2d').drawImage(image,0,0);return {width:image.width,height:image.height,pixel:Array.from(canvas.getContext('2d').getImageData(0,0,1,1).data)};},savedPhoto.file.url);
    expect(decoded.width).toBe(2);expect(decoded.height).toBe(1);expect(decoded.pixel[0]).toBeGreaterThan(200);
    const calls=(await snapshot(request)).plugin_calls.filter(c=>c.request_id===savedPhoto.request_id);expect(calls.map(c=>`${c.plugin}.${c.operation}`).sort()).toEqual(['Files.put','Image.resize']);
    const replay=await request.post('/api/products/p1/photo',{data:{photo:upload(),width:2,height:2},headers:{'Idempotency-Key':'photo-1'}});expect((await replay.json()).file.id).toBe(savedPhoto.file.id);
  });

  test('image types reject PDF, spoofed dimensions, malformed data and truncated images before plugin execution', async ({request,page}) => {
    for(const photo of [upload(Buffer.from('%PDF-1.7\nnot an image')),upload(pngFixture,{width:999}),upload(pngFixture.subarray(0,40)),{data:'not base64',name:'fake.jpg'}]) {
      const response=await request.post('/api/products/p1/photo',{data:{photo,width:2,height:2}});expect(response.status()).toBe(422);const result=await response.json();expect((await snapshot(request)).plugin_calls.filter(c=>c.request_id===result.request_id)).toEqual([]);
    }
    await page.goto('/admin');
    const jpeg=await page.evaluate(async base64=>{const image=new Image();image.src='data:image/png;base64,'+base64;await image.decode();const canvas=document.createElement('canvas');canvas.width=4;canvas.height=2;canvas.getContext('2d').drawImage(image,0,0);return canvas.toDataURL('image/jpeg').split(',')[1];},pngFixture.toString('base64'));
    const response=await request.post('/api/products/p2/photo',{data:{photo:{data:jpeg,name:'actual.jpeg'},width:2,height:2}});expect(response.status()).toBe(201);expect((await response.json()).file.media_type).toBe('image/jpeg');
    const decode=await request.post('/api/files/decode',{data:{file:upload()}});expect(decode.status()).toBe(200);expect((await decode.json()).format).toBe('PNG');
    const pdf=await request.post('/api/files/decode',{data:{file:upload(Buffer.from('%PDF-1.7\n'))}});expect(pdf.status()).toBe(422);expect((await pdf.json()).error.code).toBe('image_decode_failed');
  });

  test('file rollback leaves no file or row and native DELETE participates in transactions', async ({request}) => {
    const before=execFileSync('/usr/bin/sqlite3',[path.join(directory,'e2e.sqlite3'),'SELECT COUNT(*) FROM flow_files;']).toString().trim();
    const response=await request.post('/api/files/rollback',{data:{file:upload()}});expect(response.status()).toBe(409);
    const after=execFileSync('/usr/bin/sqlite3',[path.join(directory,'e2e.sqlite3'),'SELECT COUNT(*) FROM flow_files;']).toString().trim();expect(after).toBe(before);
    expect(execFileSync('/usr/bin/sqlite3',[path.join(directory,'e2e.sqlite3'),"SELECT COUNT(*) FROM flow_records WHERE table_name='Photos' AND id='rolled-back-file';"]).toString().trim()).toBe('0');
    const temporary=await request.post('/api/products/p1/photo',{data:{photo:upload(),width:2,height:2}});const file=(await temporary.json()).file;
    const removed=await request.delete(`/api/files/${file.id}`);expect(removed.status()).toBe(200);expect((await request.get(file.url)).status()).toBe(404);
  });

  test('WebSocket subscribers receive independent typed MQTT copies, isolate devices, suppress DUP and unsubscribe', async ({request}) => {
    const a=await wsClient();const b=await wsClient();const mqtt=await mqttClient();
    const subscribe={action:'subscribe',source:'DeviceStatus',params:{device_id:'mower1'}};
    try {
      a.send(subscribe);b.send(subscribe);expect((await a.next()).type).toBe('subscribed');expect((await b.next()).type).toBe('subscribed');
      await mqtt.publish('devices/other/status',{online:true,battery:99});
      await mqtt.publish('devices/mower1/status',{online:true,battery:97},{id:2});
      for(const ws of [a,b]){const copy=await ws.next();expect(copy.source).toBe('DeviceStatus');expect(copy.params.device_id).toBe('mower1');expect(copy.payload).toEqual({online:true,battery:97});expect(copy.received_at).toBeTruthy();}
      await mqtt.publish('devices/mower1/status',{online:true,battery:97},{id:2,dup:true});
      await mqtt.publish('devices/mower1/status',{online:false,battery:96},{id:3});expect((await a.next()).payload.battery).toBe(96);expect((await b.next()).payload.battery).toBe(96);
      a.send({...subscribe,action:'unsubscribe'});expect((await a.next()).type).toBe('unsubscribed');
      a.send({action:'subscribe',source:'DevicePosition',params:{device_id:'mower1'}});expect((await a.next()).type).toBe('subscribed');
      const invalid=await mqttClient();
      const invalidClosed=new Promise(resolve=>invalid.socket.once('close',resolve));
      invalid.socket.write(mqttPacket(0x32,Buffer.concat([mqttString('devices/mower1/status'),Buffer.from([0,1]),Buffer.from(JSON.stringify({online:true,battery:101}))])));
      await invalidClosed;
      await mqtt.publish('devices/mower1/status',{online:false,battery:95},{id:4});expect((await b.next()).payload.battery).toBe(95);
      await mqtt.publish('devices/mower1/position',{latitude:50,longitude:14},{id:5});expect((await a.next()).source).toBe('DevicePosition');
      const c=await wsClient();try{c.send({...subscribe,latest:true});expect((await c.next()).type).toBe('subscribed');expect((await c.next()).payload.battery).toBe(95);}finally{c.close();}
    } finally {a.close();b.close();mqtt.close();}
    await expect.poll(async()=> (await snapshot(request)).websocket.connections).toBe(0);
  });

  test('WebSocket contract rejects invalid subscriptions and a real Chrome client receives mower updates', async ({request,page}) => {
    const client=await wsClient();
    try {
      for(const command of [{action:'subscribe',source:'Unknown',params:{}},{action:'subscribe',source:'DeviceStatus',params:{device_id:7}},{action:'subscribe',source:'DeviceStatus',params:{device_id:'x',extra:true}},{action:'subscribe',source:'DeviceStatus',params:{device_id:'a/b'}}]){client.send(command);expect((await client.next()).type).toBe('error');}
      expect((await snapshot(request)).websocket.subscriptions).toBe(0);
    } finally {client.close();}
    await page.goto('/admin');
    await page.evaluate(async()=>{window.copies=[];window.mowerSocket=new WebSocket('ws://'+location.host+'/ws');window.mowerSocket.onmessage=event=>{const copy=JSON.parse(event.data);window.copies.push(copy);if(copy.type==='authenticated')window.mowerSocket.send(JSON.stringify({action:'subscribe',source:'DeviceStatus',params:{device_id:'browser-mower'}}));};await new Promise(resolve=>window.mowerSocket.onopen=resolve);window.mowerSocket.send(JSON.stringify({action:'authenticate',input:{token:'demo-petra'}}));});
    await expect.poll(()=>page.evaluate(()=>window.copies.some(copy=>copy.type==='subscribed'))).toBe(true);
    const mqtt=await mqttClient();try{await mqtt.publish('devices/browser-mower/status',{online:true,battery:88});}finally{mqtt.close();}
    await expect.poll(()=>page.evaluate(()=>window.copies.find(copy=>copy.type==='message')?.payload.battery)).toBe(88);
    await page.evaluate(()=>window.mowerSocket.close());
    await expect.poll(async()=> (await snapshot(request)).websocket.connections).toBe(0);
  });

  test('admin live device panel subscribes, displays mower copies and disconnects', async ({request,page}) => {
    await page.goto('/admin');
    await page.getByRole('button',{name:'Živá zařízení'}).click();
    await page.locator('#device-id').fill('panel-mower');
    await page.getByRole('button',{name:'Odebírat zprávy'}).click();
    await expect.poll(async()=> (await snapshot(request)).websocket.subscriptions).toBe(2);
    const mqtt=await mqttClient();
    try {
      await mqtt.publish('devices/panel-mower/status',{online:true,battery:73});
      await mqtt.publish('devices/panel-mower/position',{latitude:50.2,longitude:14.3},{id:2});
      await expect(page.locator('#device-status')).toContainText('73');
      await expect(page.locator('#device-position')).toContainText('50.2');
      await expect(page.locator('#device-messages')).toContainText('panel-mower');
    } finally {mqtt.close();}
    await page.getByRole('button',{name:'Odpojit',exact:true}).click();
    await expect(page.locator('#device-connection')).toHaveText('Odpojeno');
    await expect.poll(async()=> (await snapshot(request)).websocket.subscriptions).toBe(0);
  });

  test('WebSocket subscription limit is bounded and duplicate subscriptions remain idempotent', async ({request}) => {
    const client=await wsClient();
    try {
      for(let i=0;i<16;i++) {client.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'limit-'+i}});expect((await client.next()).type).toBe('subscribed');}
      client.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'limit-0'}});expect((await client.next()).type).toBe('subscribed');
      client.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'limit-16'}});expect((await client.next()).type).toBe('error');
      expect((await snapshot(request)).websocket.subscriptions).toBe(16);
      client.send({action:'unsubscribe',source:'DeviceStatus',params:{device_id:'limit-0'}});expect((await client.next()).type).toBe('unsubscribed');
      client.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'limit-16'}});expect((await client.next()).type).toBe('subscribed');
    } finally {client.close();}
    await expect.poll(async()=> (await snapshot(request)).websocket.connections).toBe(0);
  });

  test('WebSocket authenticates tokens and denies both latest and live access to other users devices', async ({request}) => {
    const unauthenticated=await wsClient(null);
    try {
      unauthenticated.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'mower1'},latest:true});expect((await unauthenticated.next()).code).toBe('authentication_required');
      for(const input of [{token:'wrong'},{token:7},{token:'demo-petra',extra:true},{}]) {unauthenticated.send({action:'authenticate',input});expect((await unauthenticated.next()).type).toBe('error');}
      unauthenticated.send({action:'authenticate',input:{token:'demo-david'}});expect((await unauthenticated.next()).type).toBe('authenticated');
      unauthenticated.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'mower1'},latest:true});expect((await unauthenticated.next()).code).toBe('forbidden');
      unauthenticated.send({action:'authenticate',input:{token:'demo-petra'}});expect((await unauthenticated.next()).code).toBe('already_authenticated');
      unauthenticated.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'mower2'}});expect((await unauthenticated.next()).type).toBe('subscribed');
      const mqtt=await mqttClient();try{await mqtt.publish('devices/mower1/status',{online:true,battery:64});await mqtt.publish('devices/mower2/status',{online:true,battery:63},{id:2});expect((await unauthenticated.next()).params.device_id).toBe('mower2');}finally{mqtt.close();}
    } finally {unauthenticated.close();}
    await expect.poll(async()=> (await snapshot(request)).websocket.connections).toBe(0);
  });

  test('revoking a grant stops an existing WebSocket subscription before the next copy is delivered', async ({request}) => {
    await request.post('/api/test-device-access',{data:{id:'revoked-grant',user_id:'u1',token:'revocable-token',device_id:'revoked-mower'}});
    const client=await wsClient('revocable-token');const mqtt=await mqttClient();
    try {
      client.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'revoked-mower'}});expect((await client.next()).type).toBe('subscribed');
      await mqtt.publish('devices/revoked-mower/status',{online:true,battery:56});expect((await client.next()).payload.battery).toBe(56);
      expect((await request.delete('/api/test-device-access/revoked-grant')).status()).toBe(200);
      await mqtt.publish('devices/revoked-mower/status',{online:true,battery:55},{id:2});expect((await client.next()).code).toBe('unauthorized');
      expect((await snapshot(request)).websocket.subscriptions).toBe(0);
      client.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'revoked-mower'},latest:true});expect((await client.next()).code).toBe('unauthorized');
    } finally {client.close();mqtt.close();}
    await expect.poll(async()=> (await snapshot(request)).websocket.connections).toBe(0);
  });

  test('per-device revocation removes one stream while the same token retains another permitted device', async ({request}) => {
    for(const device of ['a','b'])expect((await request.post('/api/test-device-access',{data:{id:'selective-'+device,user_id:'u1',token:'selective-token',device_id:'selective-'+device}})).status()).toBe(200);
    const client=await wsClient('selective-token');const mqtt=await mqttClient();
    try {
      for(const device of ['a','b']) {client.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'selective-'+device}});expect((await client.next()).type).toBe('subscribed');}
      await request.delete('/api/test-device-access/selective-a');
      await mqtt.publish('devices/selective-a/status',{online:true,battery:44});expect((await client.next()).code).toBe('forbidden');
      await mqtt.publish('devices/selective-b/status',{online:true,battery:43},{id:2});expect((await client.next()).params.device_id).toBe('selective-b');
      expect((await snapshot(request)).websocket.subscriptions).toBe(1);
    } finally {client.close();mqtt.close();}
    await expect.poll(async()=> (await snapshot(request)).websocket.connections).toBe(0);
  });

  test('SEED initializes once: revoked access stays revoked and changed credentials survive restart', async ({request}) => {
    expect((await request.delete('/api/test-device-access/david-mower2')).status()).toBe(200);
    await stopServer();await startServer();
    const rejected=await wsClient(null);
    try {rejected.send({action:'authenticate',input:{token:'demo-david'}});expect((await rejected.next()).code).toBe('unauthorized');}finally{rejected.close();}
    expect((await request.post('/api/test-device-access',{data:{id:'david-mower2',user_id:'u2',token:'changed-david-token',device_id:'mower2'}})).status()).toBe(200);
    await stopServer();await startServer();
    const changed=await wsClient('changed-david-token');
    try {changed.send({action:'subscribe',source:'DeviceStatus',params:{device_id:'mower2'},latest:true});expect((await changed.next()).type).toBe('subscribed');expect((await changed.next()).payload.battery).toBe(63);}finally{changed.close();}
    await expect.poll(async()=> (await snapshot(request)).websocket.connections).toBe(0);
  });

  test('process restart preserves SQL state, request history, plugin calls and completed idempotency keys', async ({request}) => {
    await expect.poll(async()=> (await snapshot(request)).email_jobs.every(j=>['sent','failed'].includes(j.state))).toBe(true);
    const before=await snapshot(request);
    await stopServer(); await startServer();
    const after=await snapshot(request);
    for (const key of ['products','users','orders','requests','plugin_calls','email_jobs']) expect(after[key]).toEqual(before[key]);
    expect((await request.get(savedPhoto.file.url)).status()).toBe(200);
    const device=await request.get('/api/devices/d1');const persisted=await device.json();expect(persisted.status.battery).toBe(80);expect(persisted.positions).toHaveLength(2);
    const replay=await post(request,payload(),'e2e-replay'); expect(replay.status).toBe(201); expect(replay.body.replayed).toBe(true);
    expect((await snapshot(request)).orders).toEqual(before.orders);
  });
  test('five-minute position window excludes backdated messages while last status remains available', async ({request}) => {
    // A historical database fixture avoids waiting five real minutes in an E2E suite.
    await stopServer();
    execFileSync('/usr/bin/sqlite3',[path.join(directory,'e2e.sqlite3'),"UPDATE mqtt_messages SET received_at='2020-01-01T00:00:00.000000Z' WHERE topic IN ('devices/d1/status','devices/d1/position');"]);
    await startServer();
    const response=await request.get('/api/devices/d1');expect(response.status()).toBe(200);const device=await response.json();
    expect(device.status.battery).toBe(80);expect(device.positions).toEqual([]);
    const subscriber=await mqttClient();
    try {
      subscriber.socket.write(mqttPacket(0x82,Buffer.concat([Buffer.from([0,1]),mqttString('devices/d1/status'),Buffer.from([0])])));
      expect((await subscriber.next()).header).toBe(0x90);const retained=await subscriber.next();expect(retained.header).toBe(0x31);
      const n=retained.body.readUInt16BE();expect(JSON.parse(retained.body.subarray(2+n)).battery).toBe(87);
    } finally {subscriber.close();}
  });

  test('persisted records are revalidated before being exposed as declared table or MQTT types', async ({request}) => {
    await stopServer();
    execFileSync('/usr/bin/sqlite3',[path.join(directory,'e2e.sqlite3'),`INSERT INTO mqtt_messages(source,topic,payload_json,received_at,retained) VALUES ('DevicePosition','devices/corrupt/position','{"latitude":91,"longitude":14}','2020-01-01T00:00:00.000000Z',0); INSERT INTO flow_records(table_name,id,value_json) VALUES ('Notes','corrupt','{"id":"corrupt","text":"invalid stored rating","rating":7,"tags":[]}');`]);
    await startServer();
    let response=await request.get('/api/devices/corrupt');expect(response.status()).toBe(500);expect((await response.json()).error.code).toBe('language_error');
    response=await request.get('/api/notes',{params:{filter:'{}'}});expect(response.status()).toBe(500);expect((await response.json()).error.code).toBe('language_error');
    const ws=await wsClient();
    try {ws.send({action:'subscribe',source:'DevicePosition',params:{device_id:'corrupt'},latest:true});expect((await ws.next()).type).toBe('error');expect((await snapshot(request)).websocket.subscriptions).toBe(0);} finally {ws.close();}
    expect((await request.get('/health')).status()).toBe(200);
  });

});
