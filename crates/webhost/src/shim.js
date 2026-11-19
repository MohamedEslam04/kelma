// kelma web host: injected before the page's own scripts (Page.addScriptToEvaluateOnNewDocument).
// __kelmaBridge and __kelmaTeams are CDP bindings (Runtime.addBinding): calling one sends its
// string argument to kelma as Runtime.bindingCalled.
(function () {
  "use strict";
  if (window.top !== window || window.kelma) return;
  function binding(name) {
    var f = window[name];
    if (typeof f !== "function") throw new Error("kelma host binding " + name + " is missing");
    return f;
  }
  var pending = new Map();
  var next = 1;
  // Generic bridge: window.kelma.invoke(payload) -> Promise of kelma's answer.
  window.kelma = {
    invoke: function (payload) {
      var id = next++;
      return new Promise(function (resolve, reject) {
        pending.set(id, { resolve: resolve, reject: reject });
        binding("__kelmaBridge")(
          JSON.stringify({ id: id, payload: payload === undefined ? null : payload })
        );
      });
    },
    __settle: function (id, ok, value) {
      var p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(value);
      else p.reject(new Error(String(value)));
    },
  };
  // TeamsJS: a top-level page has no parent window, so teams-js uses the "frameless" native
  // interface. Replies come back through window.onNativeMessage, which teams-js installs.
  if (window.__kelmaShim === true) {
    window.nativeInterface = {
      framelessPostMessage: function (message) {
        binding("__kelmaTeams")(String(message));
      },
    };
  }
  if (typeof window.__kelmaTitle === "string") {
    var title = window.__kelmaTitle;
    document.addEventListener("DOMContentLoaded", function () {
      document.title = title;
    });
  }
})();
