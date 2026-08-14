// mail-reply.mjs — send a mailbox message to a herm-b agent as the operator's
// laptop agent. Resolves from_agent dynamically from presence.list over the
// fleet (default) socket, then POSTs to herm-core /v1/mailbox/send.
// Usage: node mail-reply.mjs '<to_agent>' '<subject>' '<body>' [socket]
import net from 'net';

const [, , toAgent, subject, body, sockPath] = process.argv;
const sock = sockPath || '/var/lib/brn/brn.sock'; // sidecar (has presence.list + mailbox proxy)

function rpc(socketPath, method, params, id) {
  return new Promise((resolve, reject) => {
    const s = net.connect(socketPath);
    let buf = '';
    const t = setTimeout(() => { s.destroy(); reject(new Error('timeout')); }, 8000);
    s.on('connect', () => s.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n'));
    s.on('data', (d) => {
      buf += d.toString();
      if (buf.includes('\n')) { clearTimeout(t); s.destroy(); resolve(JSON.parse(buf.split('\n')[0])); }
    });
    s.on('error', (e) => { clearTimeout(t); reject(e); });
  });
}

const main = async () => {
  // resolve my agent id from presence.list on the sidecar socket
  const pres = await rpc(sock, 'presence.list', {}, '1');
  const agents = (pres.result && pres.result.agents) || [];
  const me = agents.find((a) => a.host === 'sebluair' && a.project === 'herm')
    || agents.find((a) => a.host === 'sebluair')
    || agents[0];
  if (!me) { console.error('no operator agent in presence.list'); process.exit(1); }
  const from = me.agent_id;
  const resp = await fetch('http://127.0.0.1:8787/v1/mailbox/send', {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'X-API-Token': process.env.HERM_CORE_API_TOKEN || '' },
    body: JSON.stringify({ from_agent: from, to_agent: toAgent, subject, body }),
  });
  console.log('from_agent:', from);
  console.log('status:', resp.status, await resp.text().then((t) => t.slice(0, 200)));
};
main().catch((e) => { console.error('ERR', e.message); process.exit(1); });
