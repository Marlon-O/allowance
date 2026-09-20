import { invoke, isTauri } from '@tauri-apps/api/core';
import type { View, ProviderId } from './types';
export const desktop = isTauri();
const now = Math.floor(Date.now()/1000);
let preview:View = {settings:{theme:'system',showUsed:false,alerts:false,codexEnabled:true,claudeEnabled:true},autostart:false,providers:[
 {provider:'claude',state:'ready',accountId:'demo-claude',accountLabel:'Personal account',source:'Claude Code status line',message:null,observedAt:now-120,windows:[{id:'five_hour',label:'5-hour window',usedPercent:28,resetsAt:now+9720,durationMins:300},{id:'seven_day',label:'Weekly window',usedPercent:46,resetsAt:now+299160,durationMins:10080}]},
 {provider:'codex',state:'ready',accountId:'demo-codex',accountLabel:'Personal account',source:'Codex app server',message:null,observedAt:now-30,windows:[{id:'codex:primary',label:'5-hour window',usedPercent:12,resetsAt:now+15120,durationMins:300},{id:'codex:secondary',label:'Weekly window',usedPercent:65,resetsAt:now+145800,durationMins:10080}]}
]};
export async function call<T>(command:string,args?:Record<string,unknown>):Promise<T>{
 if(desktop)return invoke<T>(command,args);
 // Browser-only visual preview. No simulated data is used in the installed app.
 switch(command){
 case 'get_state': return structuredClone(preview) as T;
 case 'update_preferences': preview.settings={...preview.settings,...{theme:args!.theme as View['settings']['theme'],showUsed:args!.showUsed as boolean,alerts:args!.alerts as boolean}};break;
 case 'set_autostart': preview.autostart=Boolean(args!.enabled);break;
 case 'connect_provider': {const id=args!.provider as ProviderId;preview.settings[id==='claude'?'claudeEnabled':'codexEnabled']=Boolean(args!.enabled);preview.providers=preview.providers.map(p=>p.provider===id?{...p,state:args!.enabled?'waiting':'disconnected',windows:[],observedAt:null,message:'Preview connection state. Open the installed app for real usage.'}:p);return 'Preview updated.' as T;}
 }
 return undefined as T;
}
