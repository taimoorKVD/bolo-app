/** Vertex shader: one fullscreen triangle, no buffers. */
export const BLOB_VS = `#version 300 es
void main(){ vec2 p=vec2((gl_VertexID<<1)&2, gl_VertexID&2); gl_Position=vec4(p*2.-1.,0.,1.); }
`;

/**
 * Shared lighting and face, plus the Bolo Blob body.
 * Ported from the v4 listening-character mockup.
 * `uFast > 0.5` skips sparkle, rim sheen, and the extra body samples.
 */
export const BLOB_FS = `#version 300 es
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
float cov(float d){ return clamp(.5-d/PX,0.,1.); }
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
