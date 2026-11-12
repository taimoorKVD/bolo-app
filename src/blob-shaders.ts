/** Vertex shader: one fullscreen triangle, no buffers. */
export const BLOB_VS = `#version 300 es
void main(){ vec2 p=vec2((gl_VertexID<<1)&2, gl_VertexID&2); gl_Position=vec4(p*2.-1.,0.,1.); }
`;

/**
 * Shared lighting and face, plus the Bolo Blob body.
 * Ported from the v4 listening-character mockup.
 * `uFast > 0.5` skips sparkle, rim sheen, and the extra body samples.
 */
export const SHARED_FS = `#version 300 es
precision highp float;
out vec4 o;
uniform vec2 uRes,uC,uSq,uLook,uPup,uLid,uLow,uTip,uTip2,uArm,uMo2,uWob,uHand;
uniform float uT,uU,uTilt,uLift,uHover,uHappy,uGlow,uErr,uSpark,uLv,uListen,uThink,uFaceY,uSlope,uCheek,uAlpha,uBlush,uBeat,uGlitch,uDone,uFast;
uniform vec4 uBrow,uMouth;
const vec3 GOLD=vec3(.851,.706,.290), CHAMP=vec3(1.,.9,.7), AMBER=vec3(.97,.56,.17), RED=vec3(.97,.36,.3);
float PX;
mat2 rot(float a){float c=cos(a),s=sin(a);return mat2(c,-s,s,c);}
float hash(float n){return fract(sin(n)*43758.5453);}
float hash12(vec2 p){vec3 p3=fract(vec3(p.xyx)*.1031);p3+=dot(p3,p3.yzx+33.33);return fract((p3.x+p3.y)*p3.z);}
float smin(float a,float b,float k){float h=clamp(.5+.5*(b-a)/k,0.,1.);return mix(b,a,h)-k*h*(1.-h);}
float sdEl(vec2 p,vec2 r){ float k0=length(p/r); float k1=length(p/(r*r)); return k0*(k0-1.)/max(k1,1e-5); }
float sdSeg(vec2 p,vec2 a,vec2 b,float ra,float rb){ vec2 pa=p-a,ba=b-a; float h=clamp(dot(pa,ba)/max(dot(ba,ba),1e-6),0.,1.); return length(pa-ba*h)-mix(ra,rb,h); }
float ss(float a,float b,float x){ float t=clamp((x-a)/(b-a),0.,1.); return t*t*(3.-2.*t); }
float vnoise(float x){float i=floor(x),f=fract(x);f=f*f*(3.-2.*f);return mix(hash(i),hash(i+1.),f);}
float sdRB(vec2 p,vec2 b,float r){vec2 q=abs(p)-b+r;return length(max(q,0.))+min(max(q.x,q.y),0.)-r;}
float cov(float d){ return clamp(.5-d/PX,0.,1.); }
float covS(float d,float s){ return clamp(.5-d/max(PX,s),0.,1.); }
void over(inout vec4 acc, vec3 c, float a){ acc=vec4(c*a,a)+acc*(1.-a); }
void glowAdd(inout vec4 acc, vec3 c, float a){ acc.rgb+=c*a; acc.a=min(1.,acc.a+a*.55); }
vec2 charP(vec2 p0){ vec2 p=p0; p.y-=uLift+uHover; p=rot(uTilt)*p; p/=uSq; return p; }
float aura(vec2 p){ vec2 q=p-vec2(0.,1.); return max(exp(-dot(q,q)*.6)-.015,0.); }
vec3 shade(float d, vec2 g, vec2 p, float T, vec3 base, vec3 sss, float gloss, float specI, float sheen){
  float x=clamp(-d/T,0.,1.); float h=sqrt(max(1.-(1.-x)*(1.-x),0.));
  float sl=(1.-x)/max(h,.08);
  vec3 N=normalize(vec3(g*sl*.85,1.));
  vec3 Lk=normalize(vec3(-.55,.66,.52)), Lf=normalize(vec3(.75,.05,.66));
  float k=dot(N,Lk);
  float dif=clamp((k+.28)/1.28,0.,1.);
  float term=clamp(1.-abs(k-.08)*3.2,0.,1.);
  vec3 col=base*(.2+.8*dif);
  if(uFast<0.5) col+=base*clamp(dot(N,Lf),0.,1.)*.2*vec3(.88,.92,1.);
  col+=sss*term*.26;
  col+=sss*exp(d/.13)*.32;
  float bounce=clamp(-N.y,0.,1.)*ss(1.3,0.,p.y);
  col+=vec3(1.,.74,.44)*bounce*.14;
  vec2 rd=normalize(N.xy+1e-4);
  float rim=pow(clamp(1.-N.z,0.,1.),2.6)*(.35+.65*clamp(dot(rd,normalize(vec2(.75,.35))),0.,1.));
  vec3 H=normalize(Lk+vec3(0,0,1)); float nh=max(dot(N,H),0.);
  col+=vec3(1.,.98,.94)*pow(nh,gloss)*specI;
  if(uFast<0.5){
    col+=vec3(1.,.94,.82)*rim*.5;
    col+=vec3(1.,.97,.9)*pow(nh,gloss*.1)*sheen;
  }
  col*=mix(.7,1.,ss(-.05,.6,p.y));
  return col;
}
float eyeShape(vec2 q, vec2 sz, float open, float low, float slope, float happy){
  float h=sz.y*max(open,.06);
  vec2 qq=q; qq.y+=sz.y*(1.-min(open,1.))*.45;
  float d=sdEl(qq,vec2(sz.x,h));
  d=max(d, qq.y+qq.x*slope-h*(1.15-.85*min(abs(slope),1.)));
  d=max(d, -qq.y-h*(1.-1.05*low));
  vec2 c=vec2(0.,-sz.y*.5); float r=sz.x*1., th=max(sz.x*.24,.022);
  vec2 a=q-c; float da=a.y>0.? abs(length(a)-r)-th : length(vec2(abs(a.x)-r,a.y))-th;
  return mix(d,da,happy);
}
void drawEye(inout vec4 acc, vec2 q, vec2 sz, float open, float low, float slope, vec3 ink, vec3 iris, float irisA, float clip){
  float d=eyeShape(q,sz,open,low,slope,uHappy);
  float a=cov(d)*clip; if(a<=0.) return;
  float vis=1.-uHappy;
  vec2 pc=uPup*sz*vec2(.34,.26);
  vec2 iq=(q-pc)/sz; float ir=length(iq*vec2(1.,.92));
  vec3 c=ink;
  c=mix(c,iris,ss(.78,.5,ir)*ss(.16,.4,ir)*irisA*vis*(.55+.45*ss(-.6,.7,-iq.y)));
  c=mix(c,ink*.55,ss(.33,.22,ir)*vis);
  float top=q.y/max(sz.y*max(open,.06),1e-3);
  c*=1.-.38*ss(.15,1.,top)*vis;
  c+=vec3(1.,.9,.8)*.1*ss(-.55,-1.,top)*vis;
  over(acc,c,a);
  float sv=vis*ss(.3,.7,open)*clip*(1.-low*.5);
  vec2 c1=sz*vec2(-.27,.33)+pc*.3, c2=sz*vec2(.3,-.28)+pc*.3;
  over(acc,vec3(1.),cov(sdEl(q-c1,vec2(sz.x*.29,sz.x*.32)))*sv*.95);
  over(acc,vec3(1.),cov(length(q-c2)-sz.x*.12)*sv*.7);
}
float mouthD(vec2 q, out float de, out vec2 b){
  q.x-=uMo2.x; q=rot(uMo2.y)*q;
  float op=uMouth.x, rd=uMouth.w, s=uMouth.z*(1.-.6*rd);
  float w=uMouth.y*(1.-.45*rd);
  b=vec2(q.x, q.y - s*q.x*q.x*6.);
  float th=.02;
  float dl=length(vec2(max(abs(b.x)-w,0.),b.y))-th;
  float hw=mix(w, max(w*.62, op*.62), rd)+th*.6;
  de=sdEl(b+vec2(0.,op*.5),vec2(hw, op*.5+th));
  return mix(dl, smin(dl,de,.02), ss(0.,.022,op));
}
void drawMouth(inout vec4 acc, vec2 q, vec3 lineC, vec3 inC, vec3 tongueC, float clip){
  float de; vec2 b; float d=mouthD(q,de,b);
  float a=cov(d)*clip; if(a<=0.) return;
  float op=uMouth.x;
  float inside=ss(.0,.03,op)*clamp(.5-de/.02,0.,1.);
  float yv=(b.y+op*.5)/(op*.5+.001);
  vec3 ic=mix(inC, tongueC, ss(-.15,-.85,yv)*.9);
  ic*=1.-.45*ss(.2,.95,yv);
  over(acc, mix(lineC, ic, inside), a);
}
void drawBrow(inout vec4 acc, vec2 q, float ang, float len, float th, vec3 c, float clip){
  q=rot(-ang)*q; q.y+=q.x*q.x*1.2; float d=sdSeg(q,vec2(-len,0.),vec2(len,0.),th,th*.7);
  over(acc,c,cov(d)*clip*.92);
}
void blush(inout vec4 acc, vec2 q, vec2 r, vec3 c, float a){ vec2 z=q/r; over(acc,c,exp(-dot(z,z)*2.2)*a); }
void sparkle(inout vec4 acc, vec2 q, float s, float amt){
  if(amt<=.001 || uFast>0.5) return;
  q/=s; float f=pow(abs(q.x),.5)+pow(abs(q.y),.5);
  float core=1.-ss(.55,1.,f); float g=exp(-dot(q,q)*1.1)*.45;
  over(acc, vec3(1.,.93,.72), clamp(core+g,0.,1.)*amt);
}
vec2 sparkPos(float t, out float front){ float a=t*2.3; front=step(0.,-sin(a)); return vec2(cos(a)*1.45, 1.15+sin(a)*.42); }
void shadow(inout vec4 acc, vec2 p0, float wid){
  float h=uLift+uHover;
  float s1=exp(-(pow(p0.x/(wid*.78),2.)+pow((p0.y+.005)/.055,2.))*2.)*.5*exp(-h*5.);
  float s2=exp(-(pow(p0.x/(wid*1.3),2.)+pow((p0.y+.02)/.17,2.))*1.6)*.32/(1.+h*1.4);
  over(acc,vec3(0.),clamp(s1+s2,0.,.75));
}
void finish(vec4 acc){
  acc*=uAlpha;
  if(uFast<0.5){
    float n=hash12(gl_FragCoord.xy+fract(uT*7.13)*391.)-.5;
    acc.rgb=max(acc.rgb+n*(2.5/255.)*acc.a,0.);
  }
  o=vec4(min(acc.rgb,vec3(acc.a)),acc.a);
}
`;

const BLOB_BODY = `
float body(vec2 p){
  vec2 q=p-vec2(0.,.95);
  float ang=atan(q.y,q.x);
  float wob=uWob.x*cos(2.*ang)+uWob.y*sin(3.*ang)+.01*sin(ang*2.+uT*1.7);
  float d=sdEl(q*vec2(1.,q.y<0.?1.1:1.),vec2(1.07,.97))-wob*.06;
  vec2 b=vec2(0.,1.74), tp=b+uTip;
  vec2 m=mix(b,tp,.5)+vec2(-(tp-b).y,(tp-b).x)*.12;
  d=smin(d,min(sdSeg(p,b,m,.15,.1),min(sdSeg(p,m,tp,.1,.07),length(p-tp)-.095)),.2);
  d=smin(d,length(p-vec2(-1.02,.58+uArm.x*.45))-.17,.12);
  d=smin(d,length(p-vec2(1.02,.58+uArm.y*.45))-.17,.12);
  return d;
}
void main(){
  PX=1./uU; vec2 p0=(gl_FragCoord.xy-uC)/uU; vec2 p=charP(p0);
  vec4 acc=vec4(0.); shadow(acc,p0,1.05);
  glowAdd(acc,mix(GOLD,AMBER,.3),aura(p)*uGlow*.5);
  float fr; vec2 sp=sparkPos(uT,fr);
  if(fr<.5) sparkle(acc,p-sp,.15,uSpark);
  float d=body(p);
  if(d<.05){
    vec2 g;
    if(uFast>0.5){
      g=normalize(vec2(dFdx(d),dFdy(d))+1e-5);
    }else{
      vec2 e=vec2(.008,0.);
      g=normalize(vec2(body(p+e.xy)-body(p-e.xy),body(p+e.yx)-body(p-e.yx))+1e-6);
    }
    vec3 base=mix(vec3(.98,.63,.19),vec3(1.,.52,.34),uErr*.3);
    vec3 col=shade(d,g,p,.78,base,vec3(.95,.3,.04),80.,1.,.12);
    col+=CHAMP*exp(-length(p-vec2(0.,.75))*1.8)*.22*(1.+uLv*.8);
    if(uFast<0.5){
      vec2 hq=rot(.55)*(p-vec2(-.5,1.48)); col+=vec3(1.)*ss(1.,.15,length(hq/vec2(.24,.11)))*.42;
    }
    over(acc,col,cov(d));
    float clip=cov(d+.04);
    vec2 f=p-vec2(0.,uFaceY)-uLook*vec2(.17,.1);
    float cy=.8+.06*uCheek;
    blush(acc,f-vec2(-.6,cy),vec2(.17,.1)*(1.+.12*uCheek),vec3(1.,.36,.28),uBlush*clip);
    blush(acc,f-vec2(.6,cy),vec2(.17,.1)*(1.+.12*uCheek),vec3(1.,.36,.28),uBlush*clip);
    vec3 ink=vec3(.11,.05,.02), iris=vec3(.55,.3,.08);
    vec2 sz=vec2(.15,.19);
    vec2 e1=f-vec2(-.35,1.07), e2=f-vec2(.35,1.07);
    over(acc,vec3(.6,.28,.04),exp(-dot(e1/vec2(.25,.28),e1/vec2(.25,.28))*2.)*.16*clip);
    over(acc,vec3(.6,.28,.04),exp(-dot(e2/vec2(.25,.28),e2/vec2(.25,.28))*2.)*.16*clip);
    drawEye(acc,e1,sz,uLid.x,uLow.x,uSlope,ink,iris,1.,clip);
    drawEye(acc,e2,sz,uLid.y,uLow.y,-uSlope,ink,iris,1.,clip);
    drawBrow(acc,f-vec2(-.36,1.37+uBrow.y),uBrow.x,.09,.025,vec3(.5,.23,.04),clip);
    drawBrow(acc,f-vec2(.36,1.37+uBrow.w),-uBrow.z,.09,.025,vec3(.5,.23,.04),clip);
    drawMouth(acc,f-vec2(0.,.73),vec3(.4,.15,.03),vec3(.3,.06,.04),vec3(.92,.42,.36),clip);
  }
  if(fr>.5) sparkle(acc,p-sp,.15,uSpark);
  finish(acc);
}
`;

export const BLOB_FS = SHARED_FS + BLOB_BODY;

export const BUDDY_FS = SHARED_FS + `
/* ===== B · MIC BUDDY ===== */
float head(vec2 p){ return sdRB(p-vec2(0.,1.05),vec2(1.,.84),.62); }
float screenD(vec2 p){ return sdRB(p-vec2(0.,1.02),vec2(.75,.55),.42); }
void main(){
  PX=1./uU; vec2 p0=(gl_FragCoord.xy-uC)/uU; vec2 p=charP(p0);
  vec4 acc=vec4(0.); shadow(acc,p0,.95);
  glowAdd(acc,mix(GOLD,AMBER,.3),aura(p)*uGlow*.45);
  float fr; vec2 sp=sparkPos(uT,fr);
  if(fr<.5) sparkle(acc,p-sp,.15,uSpark);
  vec3 led=mix(vec3(1.,.8,.38),RED*1.1,uErr*.85);
  vec2 ab=vec2(0.,1.84), at=ab+uTip; vec2 am=mix(ab,at,.5)+vec2(-(at-ab).y,(at-ab).x)*.1;
  float ds=min(sdSeg(p,ab,am,.036,.032),sdSeg(p,am,at,.032,.026));
  over(acc,mix(vec3(.5,.37,.13),vec3(1.,.86,.52),ss(-.03,.03,p.x-am.x)),cov(ds));
  float dball=length(p-at)-.1;
  float bl=.35+uLv*1.1+uBeat*1.2;
  glowAdd(acc,AMBER,max(exp(-max(dball,0.)*9.)-.01,0.)*bl*.7);
  vec3 bc=mix(vec3(1.,.74,.32),vec3(1.,.97,.86),ss(.08,-.06,length(p-at-vec2(-.03,.035))));
  over(acc,mix(bc,RED,uErr*.5)*(.9+bl*.3),cov(dball));
  for(int i=0;i<2;i++){ float sx=i==0?-1.:1.; vec2 hc=vec2(sx*1.42*(1.-.62*uHand.x),.42+(i==0?uArm.x:uArm.y)*.55+uHand.y);
    float dh=length(p-hc)-.15; if(dh<.03){ vec2 g=normalize(p-hc+1e-5); over(acc,shade(dh,g,p,.15,vec3(.95,.92,.86),vec3(1.,.78,.58),50.,.6,.06),cov(dh)); } }
  for(int i=0;i<2;i++){ float sx=i==0?-1.:1.; vec2 c=vec2(sx*1.0,1.02); float dp=sdEl(p-c,vec2(.2,.3));
    if(dp<.03){ vec2 g=normalize((p-c)/vec2(.2,.3)+1e-5); over(acc,shade(dp,g,p,.12,vec3(.95,.7,.28),vec3(1.,.45,.08),60.,.9,.1),cov(dp)); } }
  float d=head(p);
  if(d<.05){
    vec2 g; if(uFast>0.5){ g=normalize(vec2(dFdx(d),dFdy(d))+1e-5); } else { vec2 e=vec2(.008,0.); g=normalize(vec2(head(p+e.xy)-head(p-e.xy),head(p+e.yx)-head(p-e.yx))+1e-6); }
    vec3 col=shade(d,g,p,.55,vec3(.96,.93,.87),vec3(1.,.8,.6),70.,.75,.1);
    over(acc,col,cov(d));
    float sd=screenD(p);
    over(acc,vec3(.25,.2,.12),exp(-max(sd,0.)*28.)*.35*cov(d+.02));
    over(acc,mix(vec3(.58,.43,.14),vec3(1.,.87,.52),ss(-.4,.6,p.y-1.0-p.x*.3)),cov(abs(sd)-.026));
    if(sd<0.){
      vec3 sc=mix(vec3(.03,.028,.035),vec3(.085,.075,.08),ss(.4,1.6,p.y));
      over(acc,sc,cov(sd+.026));
      float clip=cov(sd+.06);
      vec2 f=p-vec2(0.,1.02+uFaceY)-uLook*vec2(.13,.08);
      f.x+=uGlitch*.07*(step(.5,fract(gl_FragCoord.y*.045+uT*9.))-.5);
      float scan=.86+.14*sin(gl_FragCoord.y*1.6);
      for(int i=0;i<2;i++){ float sx=i==0?-1.:1.; vec2 q=f-vec2(sx*.3,.1)-uPup*vec2(.03,.02);
        float op=i==0?uLid.x:uLid.y; float lw=i==0?uLow.x:uLow.y;
        float hh=max(.13*op*(1.-.5*lw),.02);
        float de=mix(sdRB(q+vec2(0.,.13*(1.-min(op,1.))*.5-.13*lw*.5),vec2(.085,hh),min(.085,hh)), eyeShape(q,vec2(.11,.12),1.,0.,0.,1.), uHappy);
        de=max(de, q.y-q.x*sx*uSlope-.15*(1.1-.8*min(abs(uSlope),1.)));
        glowAdd(acc,led,max(exp(-max(de,0.)*22.)-.02,0.)*.55*clip);
        over(acc,mix(led,vec3(1.,.97,.86),.35)*scan,cov(de)*clip);
        float bh=i==0?uBrow.y:uBrow.w; float ba=i==0?uBrow.x:-uBrow.z;
        vec2 bq=rot(-ba)*(q-vec2(0.,.3+bh)); float db=sdSeg(bq,vec2(-.08,0.),vec2(.08,0.),.02,.02);
        over(acc,led*scan,cov(db)*clip*clamp(uThink+uErr,0.,1.));
      }
      vec2 mq=f-vec2(0.,-.24);
      float de; vec2 bb; float dm=mouthD(mq,de,bb);
      // viseme LED while listening
      float vw=.05+.11*uMouth.y/.12*(1.-.45*uMouth.w), vh=.018+.36*uMouth.x;
      float dv=sdRB(mq+vec2(0.,uMouth.x*.2),vec2(vw,vh),min(vw,vh));
      dm=mix(dm,dv,uListen*(1.-uHappy));
      // thinking: loading dots
      float dd=1e3; for(int j=0;j<3;j++){ float fj=float(j); float bump=max(0.,sin(uT*6.-fj*1.1))*.05; dd=min(dd,length(mq-vec2((fj-1.)*.11,bump))-.032); }
      dm=mix(dm,dd,uThink);
      glowAdd(acc,led,max(exp(-max(dm,0.)*22.)-.02,0.)*.5*clip);
      over(acc,mix(led,vec3(1.,.97,.86),.3)*scan,cov(dm)*clip);
      float rf=ss(.0,.04,p.y-1.25+p.x*.55)*ss(.22,.1,p.y-1.25+p.x*.55);
      over(acc,vec3(1.),rf*.07*clip);
    }
  }
  if(fr>.5) sparkle(acc,p-sp,.15,uSpark);
  finish(acc);
}
`;

export const SPIRIT_FS = SHARED_FS + `
/* ===== C · GLOW SPIRIT ===== */
float body(vec2 p){
  float t=uT;
  float d=length(p-vec2(0.,1.))-(.74+.015*sin(t*2.));
  d=smin(d,length(p-vec2(-.58,.9))-(.46+.02*sin(t*1.7+1.)),.4);
  d=smin(d,length(p-vec2(.58,.9))-(.46+.02*sin(t*1.9+2.)),.4);
  d=smin(d,length(p-vec2(0.,1.36))-(.62+.02*sin(t*1.5+3.)),.5);
  vec2 tb=vec2(0.,.42), tt=tb+uTip; vec2 tm=mix(tb,tt,.55)+vec2(-(tt-tb).y,(tt-tb).x)*.15;
  d=smin(d,min(sdSeg(p,tb,tm,.36,.16),sdSeg(p,tm,tt,.16,.04)),.3);
  vec2 tf=vec2(.05,1.9)+uTip2;
  d=smin(d,length(p-tf)-.1,.34);
  d=smin(d,length(p-vec2(-1.08,.75+uArm.x*.5))-.13,.14);
  d=smin(d,length(p-vec2(1.08,.75+uArm.y*.5))-.13,.14);
  return d;
}
void main(){
  PX=1./uU; vec2 p0=(gl_FragCoord.xy-uC)/uU; vec2 p=charP(p0);
  vec4 acc=vec4(0.); shadow(acc,p0,.8);
  float d=body(p);
  vec3 gc=mix(vec3(1.,.7,.34),RED,uErr*.7);
  glowAdd(acc,gc,max(exp(-max(d,0.)*3.)-.012,0.)*(.24+uGlow*.5));
  for(int i=0;i<4;i++){ float fi=float(i); float a=uT*(.5+.17*fi)+fi*1.7; vec2 mp=vec2(cos(a)*(1.55+.25*uBeat),1.1+sin(a*1.3)*.75);
    sparkle(acc,p-mp,.05+.02*sin(uT*3.+fi)+.05*uBeat,.45+.5*uBeat); }
  float fr; vec2 sp=sparkPos(uT,fr);
  if(fr<.5) sparkle(acc,p-sp,.16,uSpark);
  if(d<.08){
    float depth=clamp(-d/.6,0.,1.);
    vec3 base=mix(vec3(1.,.72,.42),vec3(1.,.96,.88),ss(.0,.8,depth));
    base=mix(base,vec3(1.,.6,.5),uErr*.4);
    vec3 col=base*(.88+.2*ss(.2,1.9,p.y)-.1*ss(-.2,.9,p.x+.2))*(.9+.25*uGlow);
    col+=vec3(1.,.6,.3)*exp(d/.12)*.35;
    col+=vec3(1.,.98,.92)*exp(-dot(p-vec2(-.35,1.55),p-vec2(-.35,1.55))*5.)*.28;
    col*=mix(.82,1.,ss(.2,.9,p.y));
    col+=vec3(1.,.95,.85)*.04*sin(p.y*4.-uT*2.)*depth;
    over(acc,col,covS(d+.01,.045));
    float clip=cov(d+.06);
    vec2 f=p-vec2(0.,uFaceY)-uLook*vec2(.16,.1);
    float cy=.84+.06*uCheek;
    blush(acc,f-vec2(-.55,cy),vec2(.15,.09)*(1.+.12*uCheek),vec3(1.,.46,.42),uBlush*clip);
    blush(acc,f-vec2(.55,cy),vec2(.15,.09)*(1.+.12*uCheek),vec3(1.,.46,.42),uBlush*clip);
    vec3 ink=vec3(.15,.07,.07), iris=vec3(.55,.3,.18);
    drawEye(acc,f-vec2(-.31,1.08),vec2(.155,.2),uLid.x,uLow.x,uSlope,ink,iris,1.,clip);
    drawEye(acc,f-vec2(.31,1.08),vec2(.155,.2),uLid.y,uLow.y,-uSlope,ink,iris,1.,clip);
    drawBrow(acc,f-vec2(-.32,1.38+uBrow.y),uBrow.x,.075,.022,vec3(.85,.55,.35),clip*.8);
    drawBrow(acc,f-vec2(.32,1.38+uBrow.w),-uBrow.z,.075,.022,vec3(.85,.55,.35),clip*.8);
    drawMouth(acc,f-vec2(0.,.8),vec3(.42,.2,.14),vec3(.36,.11,.09),vec3(.95,.5,.45),clip);
  }
  if(fr>.5) sparkle(acc,p-sp,.16,uSpark);
  finish(acc);
}
`;

export const PEBBLE_FS = SHARED_FS + `
/* ===== D · PEBBLE ===== */
float mainB(vec2 p){
  vec2 q=p-vec2(0.,.9); q.x*=1.+.09*q.y;
  float ang=atan(q.y,q.x);
  float wob=uWob.x*cos(2.*ang)+uWob.y*sin(3.*ang);
  float ex=2.6; float r=pow(pow(abs(q.x),ex)+pow(abs(q.y)/.9,ex),1./ex);
  return (r-1.)*.88-wob*.04;
}
float fuzz(vec2 p){ vec2 q=p-vec2(0.,.9); float ang=atan(q.y,q.x); return (vnoise(ang*34.+uT*.3)-.5)*.016+(vnoise(ang*85.)-.5)*.005; }
vec2 earC(float s){ return vec2(s*.5,1.7)+(s<0.?uTip:uTip2); }
float body(vec2 p){
  float d=mainB(p);
  d=smin(d,length(p-earC(-1.))-.19,.14);
  d=smin(d,length(p-earC(1.))-.19,.14);
  d=smin(d,length(p-vec2(-.97,.52+uArm.x*.38))-.14,.1);
  d=smin(d,length(p-vec2(.97,.52+uArm.y*.38))-.14,.1);
  return d;
}
void main(){
  PX=1./uU; vec2 p0=(gl_FragCoord.xy-uC)/uU; vec2 p=charP(p0);
  vec4 acc=vec4(0.); shadow(acc,p0,1.0);
  glowAdd(acc,mix(GOLD,AMBER,.3),aura(p)*uGlow*.4);
  float fr; vec2 sp=sparkPos(uT,fr);
  if(fr<.5) sparkle(acc,p-sp,.15,uSpark);
  float d=body(p);
  if(d<.05){
    vec2 g; if(uFast>0.5){ g=normalize(vec2(dFdx(d),dFdy(d))+1e-5); } else { vec2 e=vec2(.008,0.); g=normalize(vec2(body(p+e.xy)-body(p-e.xy),body(p+e.yx)-body(p-e.yx))+1e-6); }
    vec3 base=mix(vec3(.98,.92,.83),vec3(1.,.8,.72),uErr*.3);
    vec3 col=shade(d,g,p,.8,base,vec3(1.,.58,.46),12.,.12,.05);
    col+=vec3(1.)*ss(-.06,0.,d)*.12;
    over(acc,col,covS(d+fuzz(p),.024));
    float mb=mainB(p);
    for(int i=0;i<2;i++){ float s=i==0?-1.:1.; float di=length(p-earC(s)-vec2(0.,.02))-.095;
      over(acc,vec3(1.,.68,.6),cov(di)*ss(-.02,.06,mb)*.9); }
    float clip=cov(d+.04);
    vec2 f=p-vec2(0.,uFaceY)-uLook*vec2(.2,.12);
    float cy=.76+.06*uCheek;
    blush(acc,f-vec2(-.5,cy),vec2(.15,.085)*(1.+.15*uCheek),vec3(1.,.48,.46),uBlush*1.25*clip);
    blush(acc,f-vec2(.5,cy),vec2(.15,.085)*(1.+.15*uCheek),vec3(1.,.48,.46),uBlush*1.25*clip);
    vec3 ink=vec3(.07,.055,.055);
    drawEye(acc,f-vec2(-.27,1.0),vec2(.085,.105),uLid.x,uLow.x,uSlope,ink,vec3(.32,.24,.22),.8,clip);
    drawEye(acc,f-vec2(.27,1.0),vec2(.085,.105),uLid.y,uLow.y,-uSlope,ink,vec3(.32,.24,.22),.8,clip);
    drawMouth(acc,f-vec2(0.,.75),vec3(.26,.12,.1),vec3(.32,.09,.08),vec3(.92,.48,.44),clip);
  }
  if(fr>.5) sparkle(acc,p-sp,.15,uSpark);
  finish(acc);
}
`;

export type CharacterKey = "blob" | "buddy" | "spirit" | "pebble";

export function characterFragment(key: CharacterKey): string {
  if (key === "buddy") return BUDDY_FS;
  if (key === "spirit") return SPIRIT_FS;
  if (key === "pebble") return PEBBLE_FS;
  return BLOB_FS;
}
