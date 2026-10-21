// The review window: one card per waiting request, decided only here.
const $ = (id) => document.getElementById(id);

export function card(r, decide) {
  const el = document.createElement('div');
  el.className = 'card';
  const title = document.createElement('div');
  title.className = 'label wrap';
  title.textContent = r.details.what;
  const why = document.createElement('p');
  why.className = 'sub';
  why.textContent = r.details.kind === 'site'
    ? 'Agents will be able to read pages on this site and act with your session here, until you turn it off in the side panel.'
    : `Why you're asked: it ${r.details.reason}.`;
  const row = document.createElement('div');
  row.className = 'row';
  const deny = document.createElement('button');
  deny.className = 'pill';
  deny.textContent = 'Deny';
  const allow = document.createElement('button');
  allow.className = 'primary';
  allow.textContent = r.details.kind === 'site' ? 'Allow on this site' : 'Allow once';
  deny.onclick = () => decide(r.id, false);
  allow.onclick = async () => {
    // Site access is a browser permission, granted from this click (a user
    // gesture in an extension page), never from a web page.
    if (r.details.kind === 'site') {
      const granted = await chrome.permissions.request({ origins: [`${r.details.origin}/*`] }).catch(() => false);
      if (!granted) return decide(r.id, false);
    }
    decide(r.id, true);
  };
  row.append(deny, allow);
  el.append(title, why, row);
  return el;
}

async function render() {
  const { reviews = [] } = await chrome.runtime.sendMessage({ type: 'reviews' });
  $('none').hidden = reviews.length > 0;
  $('list').replaceChildren(...reviews.map((r) => card(r, async (id, allow) => {
    await chrome.runtime.sendMessage({ type: 'review-decision', id, allow });
    render();
  })));
}

if (typeof document !== 'undefined' && document.body?.classList.contains('review')) {
  chrome.storage.onChanged.addListener(render);
  render();
}
