import http.server, socketserver, time, json, sys, threading
T0=[None]; LOADED={}; LOCK=threading.Lock()
def heavy(i): return f"""<!doctype html><title>heavy {i}</title><body style="height:5000px">tab {i}
<script>var big=[];for(var k=0;k<250000;k++)big.push({{i:k,s:'row-'+k+'-{i}'}});
fetch('/loaded?id={i}');</script></body>""".encode()
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_GET(self):
        p=self.path.split('?')[0]
        if p.startswith('/h'):
            with LOCK:
                if T0[0] is None: T0[0]=time.time()
            body=heavy(p[2:]); ct='text/html'
        elif p=='/loaded':
            i=self.path.split('id=')[-1]
            with LOCK: LOADED[i]=time.time()-T0[0]
            body=b'ok'; ct='text/plain'
        elif p=='/stats':
            with LOCK: body=json.dumps(LOADED).encode()
            ct='application/json'
        elif p=='/reset':
            with LOCK: LOADED.clear(); T0[0]=None
            body=b'ok'; ct='text/plain'
        elif p=='/blank': body=b'<!doctype html><title>blank</title>'; ct='text/html'
        else: body=b'ok'; ct='text/plain'
        self.send_response(200); self.send_header('Content-Type',ct); self.send_header('Content-Length',str(len(body))); self.send_header('Cache-Control','no-store'); self.end_headers(); self.wfile.write(body)
class S(socketserver.ThreadingMixIn,http.server.HTTPServer): daemon_threads=True
S(('127.0.0.1',8770),H).serve_forever()
