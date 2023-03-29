import { BehaviorSubject, Observable } from 'rxjs';
import { serverBridge } from './services-injector';

export interface MousePosition {
	x: number;
	y: number;
}

const mousePosition: BehaviorSubject<MousePosition> = new BehaviorSubject<MousePosition>({
	x: 0,
	y: 0
});
const mouseLeftClick: BehaviorSubject<number> = new BehaviorSubject<number>(0);
const mousePositionObservable: Observable<MousePosition> = mousePosition.asObservable();
const mouseLeftClickObservable: Observable<number> = mouseLeftClick.asObservable();

export const setMousePosition = function (position: MousePosition) {
	serverBridge.updateMouse(position.x, position.y, mouseLeftClick.value);
	mousePosition.next(position);
};

export const setMouseLeftClickState = function (state: number) {
	serverBridge.updateMouse(mousePosition.value.x, mousePosition.value.y, state);
	mouseLeftClick.next(state);
};
