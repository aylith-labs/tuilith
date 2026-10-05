(() => {
 const media=matchMedia('(prefers-color-scheme: dark)');let mode='system';try{const saved=localStorage.getItem('tuilith.theme');if(['system','light','dark'].includes(saved))mode=saved;}catch{}
 function apply(){document.documentElement.dataset.theme=mode==='system'?(media.matches?'dark':'light'):mode;}
 apply();media.addEventListener('change',apply);
 document.addEventListener('DOMContentLoaded',()=>{
  const trigger=document.getElementById('theme-button'),menu=document.getElementById('theme-menu'),options=[...menu.querySelectorAll('[role=option]')];
  function close(){menu.hidden=true;trigger.setAttribute('aria-expanded','false');}
  function render(){document.getElementById('theme-value').textContent=mode[0].toUpperCase()+mode.slice(1);options.forEach(o=>o.setAttribute('aria-selected',String(o.dataset.theme===mode)));}
  function open(){menu.hidden=false;trigger.setAttribute('aria-expanded','true');options.find(o=>o.dataset.theme===mode).focus();}
  trigger.addEventListener('click',()=>menu.hidden?open():close());trigger.addEventListener('keydown',e=>{if(['ArrowDown','ArrowUp'].includes(e.key)){e.preventDefault();open();}});
  options.forEach((option,index)=>{option.addEventListener('click',()=>{mode=option.dataset.theme;try{localStorage.setItem('tuilith.theme',mode);}catch{}apply();render();close();trigger.focus();});option.addEventListener('keydown',e=>{if(e.key==='Escape'){close();trigger.focus();}if(['ArrowDown','ArrowUp','Home','End'].includes(e.key)){e.preventDefault();const next=e.key==='Home'?0:e.key==='End'?options.length-1:(index+(e.key==='ArrowDown'?1:-1)+options.length)%options.length;options[next].focus();}});});
  document.addEventListener('click',e=>{if(!menu.contains(e.target)&&!trigger.contains(e.target))close();});render();
  const copy=document.querySelector('[data-copy]');copy.addEventListener('click',async()=>{const code=copy.previousElementSibling.textContent;const status=document.getElementById('copy-status');try{await navigator.clipboard.writeText(code);status.textContent='Copied';}catch{status.textContent='Select the example above to copy it.';}});
 });
})();
