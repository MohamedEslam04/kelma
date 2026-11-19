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
  // Device permissions: window.__kelmaGranted lists what the user granted this app (manifest
  // spellings). Everything else is refused here, before the browser is asked. The list is
  // read once and removed so the page cannot see it or change it.
  if (Array.isArray(window.__kelmaGranted)) {
    var granted = window.__kelmaGranted.slice();
    delete window.__kelmaGranted;
    var has = function (name) {
      return granted.indexOf(name) >= 0;
    };
    var refuse = function (what, name) {
      var message = "kelma: the app has not been granted the " + what + " permission";
      if (typeof DOMException === "function") return new DOMException(message, name);
      var e = new Error(message);
      e.name = name;
      return e;
    };
    var override = function (target, name, value) {
      try {
        Object.defineProperty(target, name, { value: value, configurable: true, writable: true });
      } catch (e) {
        /* a frozen target: the browser permission (CDP) still applies */
      }
    };
    var rejecting = function (what, name) {
      return function () {
        return Promise.reject(refuse(what, name));
      };
    };
    var nav = window.navigator;
    if (!has("media") && nav) {
      var md = nav.mediaDevices;
      if (md) {
        override(md, "getUserMedia", rejecting("media", "NotAllowedError"));
        override(md, "getDisplayMedia", rejecting("media", "NotAllowedError"));
      }
      var legacy = function (constraints, success, failure) {
        if (typeof failure === "function") {
          setTimeout(function () {
            failure(refuse("media", "NotAllowedError"));
          }, 0);
        }
      };
      ["getUserMedia", "webkitGetUserMedia", "mozGetUserMedia"].forEach(function (n) {
        if (n in nav) override(nav, n, legacy);
      });
    }
    if (!has("geolocation") && nav && nav.geolocation) {
      var positionError = function () {
        return {
          code: 1,
          message: "User denied Geolocation",
          PERMISSION_DENIED: 1,
          POSITION_UNAVAILABLE: 2,
          TIMEOUT: 3,
        };
      };
      var deny = function (success, failure) {
        if (typeof failure === "function") {
          setTimeout(function () {
            failure(positionError());
          }, 0);
        }
      };
      override(nav.geolocation, "getCurrentPosition", deny);
      override(nav.geolocation, "watchPosition", function (success, failure) {
        deny(success, failure);
        return 0;
      });
    }
    if (!has("notifications")) {
      if (typeof window.Notification === "function") {
        try {
          Object.defineProperty(window.Notification, "permission", {
            configurable: true,
            get: function () {
              return "denied";
            },
          });
        } catch (e) {}
        override(window.Notification, "requestPermission", function (callback) {
          if (typeof callback === "function") callback("denied");
          return Promise.resolve("denied");
        });
      }
      if (window.ServiceWorkerRegistration && window.ServiceWorkerRegistration.prototype) {
        override(
          window.ServiceWorkerRegistration.prototype,
          "showNotification",
          rejecting("notifications", "NotAllowedError")
        );
      }
    }
    if (!has("midi") && nav && "requestMIDIAccess" in nav) {
      override(nav, "requestMIDIAccess", rejecting("midi", "SecurityError"));
    }
  }
  if (typeof window.__kelmaTitle === "string") {
    var title = window.__kelmaTitle;
    document.addEventListener("DOMContentLoaded", function () {
      document.title = title;
    });
  }
})();
