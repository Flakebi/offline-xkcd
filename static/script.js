document.onkeydown = function(e) {
	e = e || window.event;
	var link = undefined;
	switch (e.keyCode) {
		case 37:
			// Left arrow
			link = document.getElementById("prev");
			break;
		case 39:
			// Right arrow
			link = document.getElementById("next");
			break;
	}
	if (link !== undefined)
		location.href = link.href;
};
