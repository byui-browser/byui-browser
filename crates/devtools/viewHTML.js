function viewHTML(){
    console.log("testing the console");
    const code_display_box = document.createElement('pre')
    const code_display_txt = document.createElement('code')
    code_display_txt.textContent = document.documentElement.outerHTML;
    code_display_box.append(code_display_txt)
    document.body.append(code_display_box)
}

//viewHTML()