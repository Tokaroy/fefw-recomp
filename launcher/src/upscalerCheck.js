import { api, esc, icon, refreshIcons, toast, errText } from './util.js';
import { t } from './i18n.js';

const fwd = (s) => String(s || '').trim().replace(/\\/g, '/');
const KIND_OF = { dlss: 'dlss', dlss_framegen: 'dlss_framegen', fsr: 'fsr', xess: 'xess' };
const ASK_OF = { dlss: 'dlss', dlss_framegen: 'dlssFg', fsr: 'fsr', xess: 'xess' };
const BUTTON_OF = { dlss: 'dlss', dlss_framegen: 'dlssFg', fsr: 'fsr', xess: 'xess' };

/**
 * What the saved settings select that the game would not find: the feature names, the missing-files detail and the one download that fixes it.
 * The Play page asks before launching, so a missing DLL is no longer discovered as a slow game.
 */
async function missingFiles(isLinux) {
  const [cfg, data] = await Promise.all([api('get_config'), api('read_settings')]);
  const g = data.game || {};
  // the runtime's own reading of the values (framegen.cpp fg_setting, temporal_dlss.cpp, extras_state.cpp): 1/2 are DLSS frame generation, 3 is FSR 3
  const up = String(g['dlss.upscaler'] ?? 'taau').trim().toLowerCase();
  const upscaler = up === 'dlaa' ? 'dlss' : up;
  const fg = String(g['dlss.frame_gen'] ?? '0').trim().toLowerCase();
  const frameGen = ['1', '2', 'dlss', 'dlssg', 'on'].includes(fg) ? 'dlss' : ['3', 'fsr3', 'fsr'].includes(fg) ? 'fsr3' : '0';
  const r = await api('upscaler_files', { fsrDir: fwd(g['upscale.fsr_dll']), dlssDir: fwd(g['upscale.dlss_dll']), xessDir: fwd(g['upscale.xess_dll']) });
  const dirs = { fsr: g['upscale.fsr_dll'], xess: g['upscale.xess_dll'], dlss: g['upscale.dlss_dll'], dlss_framegen: g['upscale.dlss_dll'] };
  const missing = [];
  const need = (feature, file, kind) => { if (!file.found) missing.push({ feature, detail: file.detail, kind }); };
  if (frameGen === 'dlss') need(t('settings.files.dlssFrameGen'), r.dlss_framegen, 'dlss_framegen');
  else if (upscaler === 'dlss') need(t('settings.files.dlss'), r.dlss, 'dlss');
  if (upscaler === 'fsr3') need(`FSR 3 ${t('settings.files.upscaling').toLowerCase()}`, r.fsr_upscaling, 'fsr');
  if (frameGen === 'fsr3' && !isLinux) need(`FSR 3 ${t('settings.files.frameGen').toLowerCase()}`, r.fsr_framegen, 'fsr');
  if (upscaler === 'xess') need(t('settings.files.xess'), r.xess, 'xess');
  return { missing, dirs, cfg, upscaler, frameGen };
}

/**
 * Resolves true when the game may be launched: nothing is missing, the player downloaded the files, switched to TAAU, or chose to play anyway.
 * Resolves false when the dialog was dismissed.
 */
export async function confirmUpscalerFiles(isLinux) {
  let st;
  try {
    st = await missingFiles(isLinux);
  } catch (e) {
    return true;
  }
  if (!st.missing.length) return true;
  // one download per kind; the frame generation set includes the plain DLSS files
  const pick = () => {
    const kinds = [...new Set(st.missing.map((m) => m.kind))];
    return kinds.includes('dlss_framegen') ? 'dlss_framegen' : kinds[0];
  };
  let kind = pick();

  return new Promise((resolve) => {
    const box = document.createElement('div');
    box.className = 'modal-backdrop';
    box.innerHTML = '<div class="modal card" role="dialog" aria-modal="true"></div>';
    document.body.appendChild(box);
    const card = box.firstElementChild;
    let asking = false;
    let timer = 0;
    let busy = false;
    const close = (v) => {
      clearTimeout(timer);
      document.removeEventListener('keydown', onKey);
      box.remove();
      resolve(v);
    };
    const onKey = (ev) => { if (ev.key === 'Escape' && !busy) close(false); };
    document.addEventListener('keydown', onKey);

    function draw(prog) {
      const list = st.missing.map((m) => `<li><strong>${esc(m.feature)}</strong><div class="faint">${esc(m.detail)}</div></li>`).join('');
      let body;
      if (prog) {
        const pct = prog.total ? Math.min(100, Math.round((prog.done / prog.total) * 100)) : 0;
        body = `<div class="banner info">${icon('download')}<div style="flex:1"><div>${esc(prog.message)}</div>${prog.total ? `<progress max="100" value="${pct}" style="width:100%"></progress>` : ''}</div><button class="btn quiet" id="m-stop">${esc(t('settings.fetch.stop'))}</button></div>`;
      } else if (asking) {
        body = `<div class="banner warn">${icon('triangle-alert')}<div style="flex:1">${esc(t(`settings.fetch.ask.${ASK_OF[kind]}`))}
          <div class="row" style="margin-top: var(--space-3)"><button class="btn primary" id="m-go">${icon('download')}${esc(t('settings.fetch.accept'))}</button><button class="btn quiet" id="m-no">${esc(t('settings.fetch.no'))}</button></div></div></div>`;
      } else {
        body = `<div class="row wrap">
          <button class="btn primary" id="m-dl">${icon('download')}${esc(t(`settings.fetch.button.${BUTTON_OF[kind]}`))}</button>
          <button class="btn" id="m-taau">${esc(t('play.files.useTaau'))}</button>
          <button class="btn quiet" id="m-anyway">${esc(t('play.files.anyway'))}</button>
          <button class="btn quiet" id="m-cancel">${esc(t('settings.fetch.no'))}</button></div>`;
      }
      card.innerHTML = `<h2>${icon('triangle-alert')}${esc(t('play.files.title'))}</h2>
        <p>${esc(t('play.files.body'))}</p><ul class="missing-list">${list}</ul>${body}`;
      refreshIcons();
      const on = (id, fn) => card.querySelector(id)?.addEventListener('click', fn);
      on('#m-dl', () => { asking = true; draw(); });
      on('#m-no', () => { asking = false; draw(); });
      on('#m-go', start);
      on('#m-stop', () => api('cancel_fetch'));
      on('#m-anyway', () => close(true));
      on('#m-cancel', () => close(false));
      on('#m-taau', useTaau);
    }

    async function useTaau() {
      const edits = [];
      if (st.upscaler === 'dlss' || st.upscaler === 'fsr3' || st.upscaler === 'xess') edits.push({ file: 'game', section: 'dlss', key: 'upscaler', value: 'taau' });
      if (st.missing.some((m) => m.kind === 'dlss_framegen') || (st.frameGen === 'fsr3' && st.missing.some((m) => m.kind === 'fsr'))) {
        edits.push({ file: 'game', section: 'dlss', key: 'frame_gen', value: '0' });
      }
      try {
        if (edits.length) await api('write_settings', { edits });
      } catch (e) {
        toast(errText(e), 'error');
        return;
      }
      close(true);
    }

    async function start() {
      asking = false;
      try {
        await api('fetch_upscaler', { kind: KIND_OF[kind], dir: fwd(st.dirs[kind]) });
      } catch (e) {
        toast(errText(e), 'error');
        draw();
        return;
      }
      busy = true;
      poll();
    }

    async function poll() {
      let s;
      try {
        s = await api('fetch_status');
      } catch (e) {
        s = { state: 'error', message: errText(e) };
      }
      if (s.state === 'running' || s.state === 'idle') {
        draw(s);
        timer = setTimeout(poll, 400);
        return;
      }
      busy = false;
      if (s.state === 'cancelled') { draw(); return; }
      toast(s.message, s.state === 'done' ? 'ok' : 'error');
      if (s.state !== 'done') { draw(); return; }
      try {
        st = await missingFiles(isLinux);
      } catch (e) {
        close(true);
        return;
      }
      if (!st.missing.length) { close(true); return; }
      kind = pick();
      draw();
    }

    draw();
  });
}
