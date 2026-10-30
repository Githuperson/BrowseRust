(function () {
  'use strict';
  if (window.__bpPause) return;
  var paused = false, replaying = false, queue = [], bytes = 0;
  var MAX_MSGS = 256, MAX_BYTES = 262144; // hard caps: the queue can never grow past these
  var overflowed = [];

  function size(d) {
    if (typeof d === 'string') return d.length * 2;
    return (d && (d.byteLength || d.size)) || 16;
  }

  function gate(o) {
    o.addEventListener('message', function (e) {
      if (replaying || !paused) return;
      // Registered before any page listener, so this also hides it from onmessage.
      e.stopImmediatePropagation();
      var sz = size(e.data);
      if (queue.length >= MAX_MSGS || bytes + sz > MAX_BYTES) {
        // Controlled fallback: close; the page sees an error on resume and reconnects.
        if (overflowed.indexOf(o) < 0) {
          overflowed.push(o);
          try { o.close(); } catch (_) {}
        }
        return;
      }
      queue.push({ o: o, d: e.data, origin: e.origin, id: e.lastEventId });
      bytes += sz;
    });
  }

  function wrap(name) {
    var N = window[name];
    if (typeof N !== 'function') return;
    var W = function (a, b) {
      var o = arguments.length > 1 ? new N(a, b) : new N(a);
      gate(o);
      return o;
    };
    W.prototype = N.prototype;
    ['CONNECTING', 'OPEN', 'CLOSING', 'CLOSED'].forEach(function (k) { if (k in N) W[k] = N[k]; });
    Object.defineProperty(window, name, { value: W, writable: true, configurable: true });
  }

  function flush() {
    var q = queue, of = overflowed;
    queue = []; bytes = 0; overflowed = [];
    replaying = true;
    try {
      for (var i = 0; i < q.length; i++) {
        try {
          q[i].o.dispatchEvent(new MessageEvent('message', { data: q[i].d, origin: q[i].origin, lastEventId: q[i].id }));
        } catch (_) {}
      }
      for (var j = 0; j < of.length; j++) {
        try { of[j].dispatchEvent(new Event('error')); } catch (_) {}
      }
    } finally { replaying = false; }
  }

  Object.defineProperty(window, '__bpPause', {
    value: function (p) { if (p) { paused = true; } else if (paused) { paused = false; flush(); } },
    enumerable: false
  });
  Object.defineProperty(window, '__bpQueued', { get: function () { return queue.length; }, enumerable: false });

  wrap('WebSocket');
  wrap('EventSource');
})();
